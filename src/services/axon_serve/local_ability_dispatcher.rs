// EasyNet CLI — `<self>.session` device-side LocalAbilityDispatcher
// =================================================================
//
// File: src/services/axon_serve/local_ability_dispatcher.rs
//
// PR-2 commit 1/N skeleton (LB-12 dispatch). Replaces
// `boot::StagingSessionDispatcher` (the `<self>.session` placeholder
// that returned a hard-coded "not-yet-wired" error for every
// inbound `SessionDispatch::Dispatch`) with a struct that holds the
// daemon's real `AbilityDispatcher` Arc threaded in at boot.
//
// Where this commit ends and commit 2/N picks up
// ----------------------------------------------
// This commit lands the boot threading + module skeleton. The
// `handle_down` method still produces the staging "not-yet-wired"
// error path so the wire-visible behaviour is unchanged from
// PR-1's `StagingSessionDispatcher`. The actual `Kernel::invoke`
// call into the local registry lands in commit 2/N once the
// boot wiring is reviewer-verified — splitting the boot plumbing
// from the dispatch logic per CTO directive 06 §3.5 "1 commit =
// 1 logical change".
//
// Why hold `AbilityDispatcher` (not `Kernel`)
// -------------------------------------------
// `AbilityDispatcher` is the dispatcher Arc the daemon's
// `easynet-daemon.rs::main` already constructs and shares with
// `Kernel::set_dispatcher`, the runtime-dispatch responder, and the
// outbound A2A path. Threading the same Arc into the session
// dispatcher means the device-side ability execution observes the
// same `LocalAbilityRegistry` state every other dispatch path
// observes — the U1 unity property the boot path already enforces.
//
// Why `handle_down` returns staging today
// ---------------------------------------
// PR-2 spec §"4-commit plan" pins commit 1/N as boot threading
// only. Commit 2/N parses `SessionDispatch::Dispatch{call_id,
// ability, args}`, builds an `Invocation` per AXIOM mapping
// (caller=peer hub, callee=self device, subject=callee, fresh
// nonce), calls `Kernel::invoke` via the dispatcher, encodes the
// terminal `Receipt` back as `SessionDispatch::Result{call_id,
// payload, terminal=true, error: Some(reason) if Failed}`, and
// sends via `outbound`. The split is reversible — if a downstream
// caller relies on the staging behaviour (none today, but future
// test scaffolding might), the rollback is a single revert.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
use crate::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use crate::pb::axon::v1::{BinaryChunk, InvokeBidiDown, InvokeBidiUp};
use crate::runtime::ability_dispatch::AbilityDispatcher;
use crate::services::axon_serve::invoke_remote_initiator::SessionDispatch;
use crate::services::axon_serve::session_initiator::{
    SessionDispatchError, SessionFrameDispatcher,
};

/// Device-side `<self>.session` dispatcher. PR-2 commit 1/N: holds
/// the boot-threaded `AbilityDispatcher` Arc but still emits the
/// staging "not-yet-wired" error reply on inbound Dispatch frames.
/// Commit 2/N replaces the staging body with a real
/// `Kernel::invoke` call routed through `dispatcher`.
#[derive(Clone)]
pub struct LocalAbilityDispatcher {
    /// The daemon's process-wide ability dispatcher. Cloned in at
    /// boot from `easynet-daemon.rs::main`'s
    /// `dispatcher_for_kernel` so this dispatcher and the rest of
    /// the daemon (Kernel::invoke, runtime-dispatch responder,
    /// outbound A2A) share one `LocalAbilityRegistry` view.
    ///
    /// Commit 1/N proves the boot threading only; commit 2/N is
    /// the change that actually calls into this dispatcher. The
    /// handler body below still takes a borrow of the field so the
    /// boot-threaded dependency is part of the compiled code path
    /// already, rather than hidden behind a lint suppression.
    dispatcher: Arc<AbilityDispatcher>,
}

impl LocalAbilityDispatcher {
    /// Construct against the boot-threaded dispatcher Arc.
    #[must_use]
    pub fn new(dispatcher: Arc<AbilityDispatcher>) -> Self {
        Self { dispatcher }
    }
}

#[async_trait::async_trait]
impl SessionFrameDispatcher for LocalAbilityDispatcher {
    /// Receive a hub-pushed Dispatch frame and reply with a
    /// staging "not-yet-wired" Result. Commit 2/N rewrites this
    /// body to invoke the held `AbilityDispatcher`.
    ///
    /// The frame parsing logic is identical to the previous
    /// `StagingSessionDispatcher::handle_down`; only the host
    /// type is new. This makes commit 1/N a no-op at the wire
    /// level — admission gate behaviour, presence-registry
    /// state, and `SessionDispatch::Result` shape all
    /// unchanged — which is the property reviewers verify when
    /// approving the boot wiring split.
    async fn handle_down(
        &self,
        frame: InvokeBidiDown,
        outbound: &mpsc::Sender<InvokeBidiUp>,
    ) -> Result<(), SessionDispatchError> {
        // Commit 1/N's contract is "the real dispatcher Arc is
        // boot-threaded all the way into the session handler".
        // Commit 2/N replaces the staging branch below with a
        // real `Kernel::invoke`, but the dependency is live now.
        let _boot_threaded_dispatcher = &self.dispatcher;

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

        let SessionDispatch::Dispatch {
            call_id,
            ability,
            args: _args,
        } = dispatch
        else {
            // Result frames flow up from the device, not down. A
            // down-stream Result is meaningless; log nothing and
            // ignore (matches the prior staging behaviour).
            return Ok(());
        };

        eprintln!(
            "[local-ability-dispatcher] received Dispatch call_id={call_id} ability={ability}; \
             replying with PR-2 commit 1/N staging not-yet-wired error \
             (commit 2/N wires real Kernel::invoke)"
        );

        let result = SessionDispatch::Result {
            call_id,
            payload: Vec::new(),
            terminal: true,
            error: Some(format!(
                "<self>.session target ability `{ability}` is not yet dispatchable on \
                 this device; PR-2 commit 1/N proves boot threading only. Real \
                 LocalAbilityRegistry dispatch ships in commit 2/N (see \
                 team-work/pr-drafts/PR-2-spec-self-session-real-handler.md)."
            )),
        };

        let payload = serde_json::to_vec(&result).map_err(|err| {
            SessionDispatchError::Other(format!("encode SessionDispatch::Result: {err}"))
        })?;

        let reply_frame = InvokeBidiUp {
            sequence: 0,
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
    use crate::runtime::gateway::NoopGateway;

    fn build_dispatcher() -> Arc<AbilityDispatcher> {
        // Construct a minimal AbilityDispatcher pointing at an
        // empty LocalAbilityRegistry + NoopGateway. Commit 1/N's
        // skeleton doesn't actually invoke the dispatcher; this
        // helper exists so commit 2/N's tests can extend it.
        let registry = Arc::new(crate::runtime::ability_dispatch::LocalAbilityRegistry::default());
        let gateway: Arc<dyn crate::runtime::gateway_api::GatewayApi> =
            Arc::new(NoopGateway::new());
        Arc::new(AbilityDispatcher::new(registry, gateway))
    }

    fn dispatch_frame(call_id: u64, ability: &str) -> InvokeBidiDown {
        let dispatch = SessionDispatch::Dispatch {
            call_id,
            ability: ability.to_string(),
            args: br#"{}"#.to_vec(),
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
    async fn dispatch_frame_replies_with_staging_error() {
        // Commit 1/N contract: skeleton produces the same staging
        // error reply that StagingSessionDispatcher did. Commit
        // 2/N replaces the body with real ability invocation;
        // this test rewrites then.
        let disp = LocalAbilityDispatcher::new(build_dispatcher());
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);

        disp.handle_down(dispatch_frame(1, "test.echo"), &tx)
            .await
            .expect("handle_down returns Ok with staging reply queued");

        let reply = rx.recv().await.expect("staging reply produced");
        let chunk = match reply.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk reply, got: {other:?}"),
        };
        let parsed: SessionDispatch =
            serde_json::from_slice(&chunk.data).expect("staging Result decodes");
        match parsed {
            SessionDispatch::Result {
                call_id,
                terminal,
                error,
                ..
            } => {
                assert_eq!(call_id, 1);
                assert!(terminal, "staging reply is terminal");
                let err = error.expect("staging reply carries error");
                assert!(
                    err.contains("commit 1/N staging not-yet-wired") || err.contains("commit 2/N"),
                    "staging error must reference commit-2/N follow-up; got: {err}"
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
        // frame. This pins the same behaviour the staging
        // dispatcher had.
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
        // No reply frame should have been queued.
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
}
