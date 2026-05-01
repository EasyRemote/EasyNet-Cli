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

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
use crate::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use crate::pb::axon::v1::{BinaryChunk, InvokeBidiDown, InvokeBidiUp};
use crate::runtime::ability_dispatch::AbilityDispatcher;
use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};
use crate::services::axon_serve::invoke_remote_initiator::{call_id_hex, SessionDispatch};
use crate::services::axon_serve::session_initiator::{
    SessionDispatchError, SessionFrameDispatcher,
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
    escalation_correlation: Option<
        Arc<crate::services::axon_serve::session_escalation::EscalationCorrelation>,
    >,
}

impl LocalAbilityDispatcher {
    /// Construct against the boot-threaded dispatcher Arc.
    #[must_use]
    pub fn new(dispatcher: Arc<AbilityDispatcher>) -> Self {
        Self {
            dispatcher,
            escalation_correlation: None,
        }
    }

    /// Builder seam: attach a device-mode escalation correlation
    /// table so inbound `RequestResult` frames complete the
    /// matching pending dispatcher future. Boot calls this in
    /// device-mode only.
    #[must_use]
    pub fn with_escalation_correlation(
        mut self,
        correlation: Arc<
            crate::services::axon_serve::session_escalation::EscalationCorrelation,
        >,
    ) -> Self {
        self.escalation_correlation = Some(correlation);
        self
    }

    fn execute_local_rpc(
        &self,
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

        let result = self.dispatcher.execute_rpc(target);

        match result {
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
        outbound: &mpsc::Sender<InvokeBidiUp>,
    ) -> Result<(), SessionDispatchError> {
        let sequence = frame.sequence;

        // Only `BinaryChunk` frames carry SessionDispatch; ignore
        // Receipt / Control frames silently (PR-1 semantics).
        let DownPayload::BinaryChunk(chunk) = frame.payload.ok_or_else(|| {
            SessionDispatchError::Other("session down frame had no payload".to_string())
        })?
        else {
            return Ok(());
        };

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
            } => (call_id, ability, args),
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

        let result = match serde_json::from_slice(&args) {
            Ok(normalized_args) => self.execute_local_rpc(&ability, normalized_args),
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
            SessionDispatch::Dispatch { .. } => {
                unreachable!("local execution never returns Dispatch")
            }
            SessionDispatch::Request { .. } | SessionDispatch::RequestResult { .. } => {
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

        let reply_frame = InvokeBidiUp {
            sequence: sequence.saturating_add(1),
            payload: Some(UpPayload::BinaryChunk(BinaryChunk {
                data: payload,
                ..BinaryChunk::default()
            })),
            ..InvokeBidiUp::default()
        };

        outbound
            .send(reply_frame)
            .await
            .map_err(|_| SessionDispatchError::Other("outbound channel closed".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::runtime::ability_dispatch::LocalAbilityRegistry;
    use crate::runtime::gateway::NoopGateway;

    fn build_dispatcher() -> Arc<AbilityDispatcher> {
        let mut registry = LocalAbilityRegistry::new();
        registry.register_rpc("test.echo", Arc::new(|args| Ok(args)));
        registry.register_rpc(
            "always.fails",
            Arc::new(|_| anyhow::bail!("simulated failure from handler")),
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

    #[tokio::test]
    async fn dispatch_frame_executes_registered_rpc_and_returns_json_payload() {
        let disp = LocalAbilityDispatcher::new(build_dispatcher());
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);

        disp.handle_down(
            dispatch_frame(1, "test.echo", br#"{"echo":"args-from-A"}"#.to_vec()),
            &tx,
        )
        .await
        .expect("handle_down returns Ok with terminal reply queued");

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

        disp.handle_down(dispatch_frame(7, "missing.ability", br#"{}"#.to_vec()), &tx)
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
    async fn malformed_args_bytes_return_terminal_error() {
        let disp = LocalAbilityDispatcher::new(build_dispatcher());
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);

        disp.handle_down(dispatch_frame(9, "test.echo", b"not-json".to_vec()), &tx)
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

        disp.handle_down(frame, &tx).await.expect("ignored cleanly");
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

        let frame = InvokeBidiDown {
            sequence: 0,
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                data: b"{not json}".to_vec(),
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        };

        let err = disp
            .handle_down(frame, &tx)
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
        std::fs::write(&target, "device-B-bytes-from-real-fs-read")
            .expect("seed temp file");

        let disp = LocalAbilityDispatcher::new(build_real_daemon_dispatcher());
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);

        let args = serde_json::json!({
            "path": target.to_string_lossy(),
            "encoding": "utf8",
        });
        let frame = dispatch_frame(
            42,
            "fs.read",
            serde_json::to_vec(&args).expect("encode args"),
        );

        disp.handle_down(frame, &tx)
            .await
            .expect("fs.read dispatches through device-mode registry");

        let reply = rx.recv().await.expect("reply produced");
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
}
