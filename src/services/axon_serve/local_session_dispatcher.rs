// EasyNet CLI — `<self>.session` device-side LocalAxonSessionDispatcher
// =================================================================
//
// File: src/services/axon_serve/local_session_dispatcher.rs
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

use crate::services::axon_serve::invoke_remote_initiator::{
    call_id_hex, SessionContentEnvelope, SessionDispatch,
};
use crate::services::axon_serve::session_initiator::{
    SessionDispatchError, SessionFrameDispatcher, SessionUpSender, SESSION_STREAM_ID,
};
use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
#[cfg(test)]
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
    escalation_correlation:
        Option<Arc<crate::services::axon_serve::session_escalation::EscalationCorrelation>>,
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
}

type LocalBidiWireKind = crate::runtime::ability_wire::AbilityBidiWireKind;

#[derive(Clone)]
struct ActiveRemoteBidi {
    ability: String,
    sender: BidiInputSender,
}

impl LocalAxonSessionDispatcher {
    fn is_json_frame_bidi(ability: &str) -> bool {
        matches!(
            crate::runtime::ability_wire::bidi_wire_kind_for(ability),
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
        }
    }

    /// Builder seam: attach a device-mode escalation correlation
    /// table so inbound `RequestResult` frames complete the
    /// matching pending dispatcher future. Boot calls this in
    /// device-mode only.
    #[must_use]
    pub fn with_escalation_correlation(
        mut self,
        correlation: Arc<crate::services::axon_serve::session_escalation::EscalationCorrelation>,
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
        let outcome = match (callee_ura, subject_ura) {
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
        };
        let request_id = outcome.invocation_id.clone();
        let (payload, error) =
            crate::runtime::axon_bridge::dispatch_shim::outcome_to_invoke_remote_result(outcome);
        Some(SessionDispatch::Result {
            call_id,
            payload,
            terminal: true,
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
                                error: None,
                                request_id: None,
                            }
                        }
                    }
                    Err(err) => {
                        sent_terminal = true;
                        SessionDispatch::Result {
                            call_id,
                            payload: Vec::new(),
                            terminal: true,
                            error: Some(format!("<self>.session: stream frame failed: {err}")),
                            request_id: None,
                        }
                    }
                };
                let terminal = matches!(dispatch, SessionDispatch::Result { terminal: true, .. });
                if Self::send_dispatch_up(&outbound, &dispatch).await.is_err() || terminal {
                    return;
                }
            }
            if !sent_terminal && !cancelled {
                let dispatch = SessionDispatch::Result {
                    call_id,
                    payload: Vec::new(),
                    terminal: true,
                    error: None,
                    request_id: None,
                };
                let _ = Self::send_dispatch_up(&outbound, &dispatch).await;
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
        let payload = serde_json::to_vec(dispatch).map_err(|err| {
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
        SessionDispatch::Result {
            call_id,
            payload: Vec::new(),
            terminal: true,
            error: Some(message.into()),
            request_id: None,
        }
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
                    error: None,
                    request_id: None,
                }))
            }
            Some("error") => {
                let reason = match (
                    value.get("code").and_then(Value::as_str),
                    value.get("message").and_then(Value::as_str),
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
                let payload = serde_json::to_vec(value).map_err(|err| {
                    SessionDispatchError::Other(format!(
                        "encode file_transfer error payload: {err}"
                    ))
                })?;
                Ok(Some(SessionDispatch::Result {
                    call_id,
                    payload,
                    terminal: true,
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
                    error: None,
                    request_id: None,
                }))
            }
            Some("exit") => Ok(Some(SessionDispatch::Result {
                call_id,
                payload: Vec::new(),
                terminal: true,
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

    fn map_remote_bidi_output(
        call_id: u64,
        ability: &str,
        value: &Value,
    ) -> Result<Option<SessionDispatch>, SessionDispatchError> {
        if ability == crate::runtime::agents::pty_attach_ability::ABILITY_PTY_SESSION_ATTACH {
            return Self::map_remote_pty_output(call_id, value);
        }
        if Self::is_json_frame_bidi(ability) {
            let terminal = matches!(
                value.get("type").and_then(Value::as_str),
                Some("closed") | Some("error")
            );
            let payload = serde_json::to_vec(value).map_err(|err| {
                SessionDispatchError::Other(format!("plugin JSON-frame bidi encode failed: {err}"))
            })?;
            return Ok(Some(SessionDispatch::Result {
                call_id,
                payload,
                terminal,
                error: None,
                request_id: None,
            }));
        }
        Self::map_remote_file_transfer_output(call_id, value)
    }

    async fn open_remote_bidi(
        &self,
        call_id: u64,
        ability: &str,
        args: Vec<u8>,
        args_content_envelope: SessionContentEnvelope,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        if !crate::runtime::ability_wire::is_bidi_wire_ability(ability) {
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

        let handle = match runtime.invoke_bidi_async(ability, args, None, None).await {
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
        tokio::spawn(async move {
            while let Some(frame_result) = handler_out_rx.next_frame().await {
                let frame = match frame_result {
                    Ok(frame) => frame,
                    Err(err) => {
                        let dispatch = LocalAxonSessionDispatcher::file_transfer_terminal_error(
                            call_id,
                            format!("<self>.session: remote file_transfer frame failed: {err}"),
                        );
                        let _ = LocalAxonSessionDispatcher::send_dispatch_up(&outbound, &dispatch)
                            .await;
                        break;
                    }
                };
                let mapped = if frame.payload.is_empty() {
                    None
                } else if LocalAxonSessionDispatcher::is_json_frame_bidi(&ability_owned)
                    && !frame.content_type.is_empty()
                    && frame.content_type != "application/json"
                {
                    Some(SessionDispatch::Result {
                        call_id,
                        payload: frame.payload,
                        terminal: frame.terminal,
                        error: None,
                        request_id: None,
                    })
                } else {
                    match serde_json::from_slice::<Value>(&frame.payload) {
                        Ok(value) => {
                            match LocalAxonSessionDispatcher::map_remote_bidi_output(
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
                if LocalAxonSessionDispatcher::send_dispatch_up(&outbound, &mapped)
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
            if Self::is_json_frame_bidi(&active.ability) {
                json!({"type": "close", "reason": "bidi_eof"})
            } else {
                json!({"type": "eof"})
            }
        } else if active.ability
            == crate::runtime::agents::pty_attach_ability::ABILITY_PTY_SESSION_ATTACH
            || Self::is_json_frame_bidi(&active.ability)
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

        let dispatch: SessionDispatch = serde_json::from_slice(&chunk.data).map_err(|err| {
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
        let (call_id, callee_ura, subject_ura, ability, args, args_content_envelope) =
            match dispatch {
                SessionDispatch::Dispatch {
                    call_id,
                    callee_ura,
                    subject_ura,
                    ability,
                    args,
                    args_content_envelope,
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
                    )
                }
                SessionDispatch::BidiOpen {
                    call_id,
                    ability,
                    args,
                    args_content_envelope,
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
                        .open_remote_bidi(call_id, &ability, args, args_content_envelope, outbound)
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
            Ok(SessionDispatch::Result {
                call_id,
                payload: Vec::new(),
                terminal: true,
                error: Some(reason),
                request_id: None,
            })
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
                Err(reason) => Ok(SessionDispatch::Result {
                    call_id,
                    payload: Vec::new(),
                    terminal: true,
                    error: Some(reason),
                    request_id: None,
                }),
            }
        } else if let Some(axon_result) = self
            .try_dispatch_via_axon(
                call_id,
                callee_ura.as_deref(),
                subject_ura.as_deref(),
                &ability,
                &args,
            )
            .await
        {
            // ── Phase 5f: Axon-only session dispatch ───────────────
            //
            // When the shared `LocalRuntime` hosts this ability, route
            // through `invoke_async`. The runtime fires the wired
            // `LedgerSink` on the terminal event. If the runtime does
            // not host the ability, we return a terminal error below;
            // there is no legacy RPC fallback from session frames.
            Ok(axon_result)
        } else {
            Ok(SessionDispatch::Result {
                call_id,
                payload: Vec::new(),
                terminal: true,
                error: Some(format!(
                    "<self>.session: ability `{ability}` is not registered in Axon LocalRuntime"
                )),
                request_id: None,
            })
        }?;

        let result = match result {
            SessionDispatch::Result {
                payload,
                terminal,
                error,
                request_id,
                ..
            } => SessionDispatch::Result {
                call_id,
                payload,
                terminal,
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
                payload,
                request_id: _,
            } => {
                assert_eq!(call_id, 1);
                assert!(terminal, "RPC reply is terminal");
                assert_eq!(error, None, "test.echo must succeed");
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
            "device.camera.snapshot",
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
                ability: "device.camera.snapshot".to_string(),
                args: b"{}".to_vec(),
                args_content_envelope: SessionContentEnvelope::plaintext_json(),
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
            "device.screen.subscribe",
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
                ability: "device.screen.subscribe".to_string(),
                args: b"{}".to_vec(),
                args_content_envelope: SessionContentEnvelope::plaintext_json(),
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
            SessionDispatch::Result { error, payload, .. } => {
                assert!(payload.is_empty(), "runtime miss carries no payload");
                let err = error.expect("runtime miss must surface an error");
                assert!(err.contains("test.echo"));
                assert!(err.contains("LocalRuntime"));
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
                payload,
                request_id: _,
            } => {
                assert_eq!(call_id, 7);
                assert!(terminal, "error reply must be terminal");
                assert!(payload.is_empty(), "failed dispatch carries no payload");
                let err = error.expect("missing ability must surface error");
                assert!(err.contains("missing.ability"));
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
                payload,
                request_id: _,
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
                payload,
                request_id: _,
            } => {
                assert_eq!(call_id, 9);
                assert!(terminal, "malformed args error must be terminal");
                assert!(payload.is_empty());
                let err = error.expect("error message required");
                assert!(err.contains("payload not JSON"));
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
                payload,
                request_id: _,
            } => {
                assert_eq!(call_id, 19);
                assert!(terminal);
                assert!(payload.is_empty());
                let err = error.expect("encrypted dispatch must fail closed");
                assert!(err.contains("encrypted args"));
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
            "path": target.to_string_lossy(),
            "encoding": "utf8",
        });
        let frame = dispatch_frame(
            42,
            "device.fs.read",
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
                ability: crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER
                    .to_string(),
                args: serde_json::to_vec(&json!({
                    "mode": "upload",
                    "path": target.to_string_lossy(),
                }))
                .expect("encode args"),
                args_content_envelope: SessionContentEnvelope::plaintext_json(),
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
        let mapped = LocalAxonSessionDispatcher::map_remote_bidi_output(
            91,
            "device.remote_desktop.attach",
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
                request_id: _,
            } => {
                assert_eq!(call_id, 91);
                assert!(!terminal);
                assert_eq!(error, None);
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
        let mapped = LocalAxonSessionDispatcher::map_remote_bidi_output(
            92,
            "device.remote_desktop.attach",
            &json!({
                "type": "closed",
                "reason": "client_closed",
            }),
        )
        .expect("map succeeds")
        .expect("closed forwards");

        match mapped {
            SessionDispatch::Result { terminal, .. } => assert!(terminal),
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
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
                ability: crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER
                    .to_string(),
                args: serde_json::to_vec(&json!({
                    "mode": "download",
                    "path": target.to_string_lossy(),
                }))
                .expect("encode args"),
                args_content_envelope: SessionContentEnvelope::plaintext_json(),
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
}
