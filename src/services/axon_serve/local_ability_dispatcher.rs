// EasyNet CLI — `<self>.session` device-side LocalAbilityDispatcher
// =================================================================
//
// File: src/services/axon_serve/local_ability_dispatcher.rs
//
// PR-2 commit 2/N. Replaces `boot::StagingSessionDispatcher` (the
// `<self>.session` placeholder that returned a hard-coded
// "not-yet-wired" error for every inbound
// `SessionDispatch::Dispatch`) with a dispatcher that executes local
// RPC abilities through the daemon's boot-threaded
// `AbilityDispatcher` Arc.
//
// Historical split
// ----------------
// PR-2 commit 1/N landed the boot threading only: the daemon now
// passes one process-wide `AbilityDispatcher` Arc into the
// device-side session handler. This file is the follow-up that
// spends that dependency for real work: decode
// `SessionDispatch::Dispatch{call_id, ability, args}`, route the
// ability through `AbilityDispatcher::execute_rpc`, then encode the
// outcome back as `SessionDispatch::Result`.
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
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
#[cfg(test)]
use crate::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
#[cfg(test)]
use crate::pb::axon::v1::InvokeBidiUp;
use crate::pb::axon::v1::{BinaryChunk, InvokeBidiDown};
use crate::runtime::ability_dispatch::AbilityDispatcher;
use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};
use crate::services::axon_serve::invoke_remote_initiator::{call_id_hex, SessionDispatch};
use crate::services::axon_serve::session_initiator::{
    SessionDispatchError, SessionFrameDispatcher, SessionUpSender, SESSION_STREAM_ID,
};

/// Device-side `<self>.session` dispatcher. Holds the boot-threaded
/// `AbilityDispatcher` Arc and executes inbound Dispatch frames
/// against the local RPC registry, returning the result payload or
/// typed failure over the existing `SessionDispatch::Result` wire
/// shape.
#[derive(Clone)]
pub struct LocalAbilityDispatcher {
    /// The daemon's process-wide ability dispatcher. Cloned in at
    /// boot from `easynet-daemon.rs::main`'s
    /// `dispatcher_for_kernel` so this dispatcher and the rest of
    /// the daemon (runtime-dispatch responder, outbound A2A, future
    /// `Kernel::invoke` callers) share one `LocalAbilityRegistry`
    /// view.
    dispatcher: Arc<AbilityDispatcher>,
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
    /// call_id. Today this is only used for `fleet.file_transfer`:
    /// hub opens the local bidi on the device, then subsequent
    /// `SessionDispatch::BidiInput` frames route through this table.
    remote_bidi_sessions: Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>,
}

impl LocalAbilityDispatcher {
    /// Construct against the boot-threaded dispatcher Arc.
    #[must_use]
    pub fn new(dispatcher: Arc<AbilityDispatcher>) -> Self {
        Self {
            dispatcher,
            escalation_correlation: None,
            remote_bidi_sessions: Arc::new(Mutex::new(HashMap::new())),
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

    fn execute_local_rpc(
        &self,
        ability: &str,
        normalized_args: serde_json::Value,
    ) -> Result<SessionDispatch, SessionDispatchError> {
        Self::execute_local_rpc_blocking(&self.dispatcher, ability, normalized_args)
    }

    /// Static variant that takes the dispatcher Arc by reference so
    /// it can be moved into `tokio::task::spawn_blocking`. Both this
    /// and `execute_local_rpc` produce identical bytes; this one
    /// exists so `handle_down` can keep blocking ability handlers
    /// (e.g. `process.exec`, `shell.run`) on the blocking pool
    /// thread, where `Handle::current().block_on(...)` is safe.
    fn execute_local_rpc_blocking(
        dispatcher: &Arc<AbilityDispatcher>,
        ability: &str,
        normalized_args: serde_json::Value,
    ) -> Result<SessionDispatch, SessionDispatchError> {
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ability.to_string(),
            normalized_args,
            call_mode: CallMode::Rpc,
            subject: None,
        };

        match dispatcher.execute_rpc(target) {
            Ok(value) => {
                let payload = serde_json::to_vec(&value).map_err(|err| {
                    SessionDispatchError::Other(format!(
                        "<self>.session: encode ability `{ability}` response JSON: {err}"
                    ))
                })?;
                Ok(SessionDispatch::Result {
                    call_id: 0,
                    payload,
                    terminal: true,
                    error: None,
                })
            }
            Err(err) => Ok(SessionDispatch::Result {
                call_id: 0,
                payload: Vec::new(),
                terminal: true,
                error: Some(format!("<self>.session: ability `{ability}` failed: {err}")),
            }),
        }
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
        }
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
                }))
            }
            Some("warn") => Ok(None),
            Some(other) => Err(SessionDispatchError::Other(format!(
                "unknown file_transfer handler frame type {other:?}"
            ))),
            None => Ok(None),
        }
    }

    async fn open_remote_file_transfer_bidi(
        &self,
        call_id: u64,
        ability: &str,
        args: Vec<u8>,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        if ability != crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER {
            return Self::send_dispatch_up(
                outbound,
                &Self::file_transfer_terminal_error(
                    call_id,
                    format!("remote bidi ability `{ability}` is not wired on <self>.session"),
                ),
            )
            .await;
        }

        let normalized_args = match serde_json::from_slice::<Value>(&args) {
            Ok(args) => args,
            Err(err) => {
                return Self::send_dispatch_up(
                    outbound,
                    &Self::file_transfer_terminal_error(
                        call_id,
                        format!(
                            "<self>.session: remote file_transfer received non-JSON args bytes: {err}"
                        ),
                    ),
                )
                .await;
            }
        };

        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ability.to_string(),
            normalized_args,
            call_mode: CallMode::Bidi,
            subject: None,
        };

        let bidi_source = match self.dispatcher.execute_bidi(target) {
            Ok(source) => source,
            Err(err) => {
                return Self::send_dispatch_up(
                    outbound,
                    &Self::file_transfer_terminal_error(
                        call_id,
                        format!("<self>.session: remote file_transfer open failed: {err}"),
                    ),
                )
                .await;
            }
        };

        let crate::runtime::ability_dispatch::BidiSource {
            to_client: handler_in_tx,
            from_client: mut handler_out_rx,
        } = bidi_source;

        {
            let mut guard = match self.remote_bidi_sessions.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.insert(call_id, handler_in_tx);
        }

        let sessions = Arc::clone(&self.remote_bidi_sessions);
        let outbound = outbound.clone();
        tokio::spawn(async move {
            while let Some(value) = handler_out_rx.recv().await {
                let mapped = match LocalAbilityDispatcher::map_remote_file_transfer_output(
                    call_id, &value,
                ) {
                    Ok(Some(dispatch)) => dispatch,
                    Ok(None) => continue,
                    Err(err) => LocalAbilityDispatcher::file_transfer_terminal_error(
                        call_id,
                        format!("<self>.session: remote file_transfer output map failed: {err}"),
                    ),
                };
                let terminal = matches!(mapped, SessionDispatch::Result { terminal: true, .. });
                if LocalAbilityDispatcher::send_dispatch_up(&outbound, &mapped)
                    .await
                    .is_err()
                {
                    break;
                }
                if terminal {
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

    async fn forward_remote_file_transfer_input(
        &self,
        call_id: u64,
        payload: Vec<u8>,
        eof: bool,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        let sender = {
            let mut guard = match self.remote_bidi_sessions.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            let sender = guard.get(&call_id).cloned();
            if eof {
                guard.remove(&call_id);
            }
            sender
        };

        let Some(sender) = sender else {
            return Self::send_dispatch_up(
                outbound,
                &Self::file_transfer_terminal_error(
                    call_id,
                    format!("remote file_transfer call_id={call_id} is not open on this device"),
                ),
            )
            .await;
        };

        let frame = if eof {
            json!({"type": "eof"})
        } else {
            json!({"type": "chunk", "data": B64.encode(payload)})
        };
        if sender.send(frame).await.is_err() {
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
                    format!("remote file_transfer call_id={call_id} input channel closed"),
                ),
            )
            .await;
        }
        Ok(())
    }
}

/// Recover a printable message from a `Box<dyn Any + Send>` panic
/// payload. Tokio's `JoinError::try_into_panic` returns the raw
/// payload Rust handed to `std::panic::catch_unwind`; the two
/// canonical shapes are `&'static str` (`panic!("...")`) and
/// `String` (`panic!("{}", value)`). Anything else collapses to a
/// type-name placeholder so the operator at least sees that a
/// panic occurred even if the payload type is exotic.
fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "panic payload not recoverable".to_string()
}

#[async_trait::async_trait]
impl SessionFrameDispatcher for LocalAbilityDispatcher {
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
        eprintln!(
            "[local-ability-dispatcher] handle_down: BinaryChunk stream_id={} data_bytes={}",
            chunk.stream_id,
            chunk.data.len()
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
        let (call_id, ability, args) = match dispatch {
            SessionDispatch::Dispatch {
                call_id,
                ability,
                args,
            } => {
                eprintln!(
                    "[local-ability-dispatcher] received Dispatch frame \
                     call_id={call_id} ability={ability} args_bytes={}",
                    args.len()
                );
                (call_id, ability, args)
            }
            SessionDispatch::BidiOpen {
                call_id,
                ability,
                args,
            } => {
                eprintln!(
                    "[local-ability-dispatcher] received BidiOpen frame \
                     call_id={call_id} ability={ability} args_bytes={}",
                    args.len()
                );
                return self
                    .open_remote_file_transfer_bidi(call_id, &ability, args, outbound)
                    .await;
            }
            SessionDispatch::BidiInput {
                call_id,
                payload,
                eof,
            } => {
                return self
                    .forward_remote_file_transfer_input(call_id, payload, eof, outbound)
                    .await;
            }
            SessionDispatch::RequestResult { call_id, outcome } => {
                if let Some(correlation) = self.escalation_correlation.as_ref() {
                    let id_hex = call_id_hex(&call_id);
                    let fired = correlation.complete(call_id, outcome);
                    if !fired {
                        eprintln!(
                            "[local-ability-dispatcher] inbound RequestResult \
                             call_id={id_hex} did not match a pending entry; \
                             dropping (caller may have timed out, or hub double-replied)"
                        );
                    } else {
                        eprintln!(
                            "[local-ability-dispatcher] inbound RequestResult \
                             call_id={id_hex} matched pending entry; completed"
                        );
                    }
                } else {
                    eprintln!(
                        "[local-ability-dispatcher] inbound RequestResult on a \
                         hub-mode daemon (no escalation_correlation wired); \
                         ignoring"
                    );
                }
                return Ok(());
            }
            SessionDispatch::Result { .. } | SessionDispatch::Request { .. } => {
                return Ok(());
            }
        };

        let result = match serde_json::from_slice::<serde_json::Value>(&args) {
            Ok(normalized_args) => {
                // LB-60 Gap 5a: execute_local_rpc invokes ability
                // handlers on the calling thread; AXIOM Tier 2.5
                // handlers like process.exec / shell.run wrap an
                // async `execute()` call in `Handle::current().
                // block_on()` on the assumption that the
                // ability registry's `register_rpc` runs them
                // inside `tokio::task::spawn_blocking`. That
                // invariant holds for direct CLI invocations,
                // but the cross-hub forward_invoke path (LB-57
                // Option A) drives `handle_down` from a tokio
                // worker thread — `block_on` from there panics
                // ("Cannot start a runtime from within a
                // runtime"). Wrapping here keeps every handler
                // on the blocking pool regardless of the
                // dispatch entrypoint without changing the
                // handler API.
                let dispatcher = Arc::clone(&self.dispatcher);
                let ability_for_blocking = ability.clone();
                let join = tokio::task::spawn_blocking(move || {
                    Self::execute_local_rpc_blocking(
                        &dispatcher,
                        &ability_for_blocking,
                        normalized_args,
                    )
                })
                .await;
                match join {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(err)) => Err(err),
                    Err(join_err) if join_err.is_panic() => {
                        // LB-60 Gap 5b: handler panic must surface
                        // as a typed Result error frame instead of
                        // tearing down the session bidi. Stringify
                        // the panic so downstream operators can
                        // grep the cause.
                        let panic_msg = match join_err.try_into_panic() {
                            Ok(payload) => panic_payload_to_string(payload),
                            Err(_) => "panic payload not recoverable".to_string(),
                        };
                        Ok(SessionDispatch::Result {
                            call_id,
                            payload: Vec::new(),
                            terminal: true,
                            error: Some(format!(
                                "<self>.session: ability `{ability}` panicked: {panic_msg}"
                            )),
                        })
                    }
                    Err(join_err) => Ok(SessionDispatch::Result {
                        call_id,
                        payload: Vec::new(),
                        terminal: true,
                        error: Some(format!(
                            "<self>.session: ability `{ability}` execution task \
                             cancelled or aborted: {join_err}"
                        )),
                    }),
                }
            }
            Err(err) => Ok(SessionDispatch::Result {
                call_id,
                payload: Vec::new(),
                terminal: true,
                error: Some(format!(
                    "<self>.session: ability `{ability}` received non-JSON args bytes: {err}"
                )),
            }),
        }?;

        let result = match result {
            SessionDispatch::Result {
                payload,
                terminal,
                error,
                ..
            } => SessionDispatch::Result {
                call_id,
                payload,
                terminal,
                error,
            },
            SessionDispatch::Dispatch { .. } | SessionDispatch::BidiOpen { .. } => {
                unreachable!("local execution never returns Dispatch")
            }
            SessionDispatch::BidiInput { .. }
            | SessionDispatch::Request { .. }
            | SessionDispatch::RequestResult { .. } => {
                // PR-N6 wire shape (C2) added these for the
                // device → hub forward_invoke escalation path.
                // LocalAbilityDispatcher only handles
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
        eprintln!(
            "[local-ability-dispatcher] sending Result frame up bidi: \
             call_id={call_id} payload_bytes={payload_len}"
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
            eprintln!(
                "[local-ability-dispatcher] FAILED to send Result frame up bidi for call_id={call_id} — outbound channel closed"
            );
        } else {
            eprintln!(
                "[local-ability-dispatcher] Result frame sent up bidi successfully for call_id={call_id}"
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

    use crate::runtime::ability_dispatch::LocalAbilityRegistry;
    use crate::runtime::gateway::NoopGateway;

    fn build_dispatcher() -> Arc<AbilityDispatcher> {
        let mut registry = LocalAbilityRegistry::new();
        registry.register_rpc("test.echo", Arc::new(|args| Ok(args)));
        registry.register_rpc(
            "always.fails",
            Arc::new(|_| anyhow::bail!("simulated failure from handler")),
        );
        // LB-60 Gap 5b regression: a handler that panics. Cross-hub
        // forward_invoke must surface the panic as a typed Result
        // error frame, not by tearing down the session bidi.
        registry.register_rpc(
            "always.panics",
            Arc::new(|_| panic!("simulated handler panic for LB-60 regression")),
        );
        let gateway: Arc<dyn crate::runtime::gateway_api::GatewayApi> =
            Arc::new(NoopGateway::new());
        Arc::new(AbilityDispatcher::new(Arc::new(registry), gateway))
    }

    fn dispatch_frame(call_id: u64, ability: &str, args: Vec<u8>) -> InvokeBidiDown {
        let dispatch = SessionDispatch::Dispatch {
            call_id,
            ability: ability.to_string(),
            args,
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
        let disp = LocalAbilityDispatcher::new(build_dispatcher());
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
    async fn unregistered_ability_returns_terminal_error() {
        let disp = LocalAbilityDispatcher::new(build_dispatcher());
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
    async fn handler_panic_surfaces_as_terminal_error_not_session_teardown() {
        // LB-60 Gap 5b regression: in the cross-hub forward_invoke
        // path, `handle_down` runs from a tokio worker thread and
        // some baseline-locomotion handlers (process.exec / shell.run)
        // call `tokio::runtime::Handle::current().block_on(...)`.
        // That panics from a worker thread; before Gap 5a/5b the
        // panic propagated through `handle_down`, killed the worker,
        // tore down the device-mode session bidi, and the caller
        // saw `target_offline` instead of a typed handler error.
        //
        // This test pins the post-Gap 5b contract: a handler panic
        // is caught at the spawn_blocking boundary and surfaced as a
        // terminal `SessionDispatch::Result { error: Some(...) }`
        // frame whose payload names the panicking ability. The
        // dispatcher remains usable for follow-up dispatches.
        let disp = LocalAbilityDispatcher::new(build_dispatcher());
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            dispatch_frame(11, "always.panics", b"{}".to_vec()),
            &session_tx,
        )
        .await
        .expect("handler panic must NOT propagate; handle_down stays Ok");

        let reply = rx.recv().await.expect("panic recovery still emits a reply");
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
            } => {
                assert_eq!(call_id, 11);
                assert!(terminal, "panic recovery reply must be terminal");
                assert!(payload.is_empty(), "panicked dispatch carries no payload");
                let err = error.expect("panic must surface as Result.error");
                assert!(
                    err.contains("always.panics"),
                    "error must name the panicking ability; got: {err}"
                );
                assert!(
                    err.contains("panicked"),
                    "error must mark the failure mode as a panic; got: {err}"
                );
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }

        // Dispatcher must remain usable for follow-up calls — the
        // panic was contained, not a fatal trap.
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
                assert!(
                    error.is_none(),
                    "post-panic test.echo must succeed; got error: {error:?}"
                );
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn malformed_args_bytes_return_terminal_error() {
        let disp = LocalAbilityDispatcher::new(build_dispatcher());
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
            } => {
                assert_eq!(call_id, 9);
                assert!(terminal, "malformed args error must be terminal");
                assert!(payload.is_empty());
                let err = error.expect("error message required");
                assert!(err.contains("non-JSON args bytes"));
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
        let disp = LocalAbilityDispatcher::new(build_dispatcher());
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        let bogus = SessionDispatch::Result {
            call_id: 42,
            payload: Vec::new(),
            terminal: true,
            error: None,
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
        let disp = LocalAbilityDispatcher::new(build_dispatcher());
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
    // against the same `Arc<AbilityDispatcher>` the daemon binary
    // constructs via `agents::build_registry_for_daemon` →
    // `build_registry_with_services`. That path registers the
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
    // a tempfile through `LocalAbilityDispatcher::handle_down`.
    // Asserts the up-channel receives a terminal Result frame
    // whose payload decodes to an `fs.read` response containing
    // the file's bytes.

    fn build_real_daemon_dispatcher() -> Arc<AbilityDispatcher> {
        use crate::runtime::execution::discuss::DiscussService;
        use crate::runtime::execution::loop_instance::LoopService;
        use crate::runtime::execution::permission::PermissionService;
        use crate::runtime::execution::schedule::ScheduleService;
        use crate::runtime::execution::session::SessionService;
        use crate::runtime::gateway::NoopGateway;
        let registry = crate::runtime::agents::build_registry_with_services(
            Arc::new(SessionService::new()),
            Arc::new(PermissionService::new()),
            Arc::new(DiscussService::new()),
            Arc::new(ScheduleService::new()),
            Arc::new(LoopService::new()),
            &Default::default(),
            Arc::new(Vec::new()),
        );
        let gateway: Arc<dyn crate::runtime::gateway_api::GatewayApi> =
            Arc::new(NoopGateway::new());
        Arc::new(AbilityDispatcher::new(registry, gateway))
    }

    #[tokio::test]
    async fn device_mode_dispatcher_executes_fs_read_through_baseline_locomotion_registry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("hello.txt");
        std::fs::write(&target, "device-B-bytes-from-real-fs-read").expect("seed temp file");

        let disp = LocalAbilityDispatcher::new(build_real_daemon_dispatcher());
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        let args = serde_json::json!({
            "path": target.to_string_lossy(),
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

        let disp = LocalAbilityDispatcher::new(build_real_daemon_dispatcher());
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_file_transfer_download_round_trips_over_session_bidi_frames() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("download-to-hub.bin");
        let bytes = b"remote-download-bytes-from-device";
        std::fs::write(&target, bytes).expect("seed file");

        let disp = LocalAbilityDispatcher::new(build_real_daemon_dispatcher());
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
