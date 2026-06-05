// EasyNet CLI — invocation_transport — device-mode `forward_invoke` escalation
// =====================================================================
//
// File: src/services/invocation_transport/session_escalation.rs
// Description: Lets a device-mode daemon's `dispatch_federation_
//              forward_invoke` send a `SessionDispatch::Request`
//              frame up the long-lived `<self>.session` bidi to its
//              hub, then `await` the matching `RequestResult` on a
//              `tokio::sync::oneshot` channel.
//
// Why this module exists
// ----------------------
// Device-mode daemons only dial outbound `<self>.session` and never
// accept inbound bidi; their local `PresenceRegistry` is empty by
// construction (no peer ever calls `presence.insert` on a
// device-mode daemon's registry). The hub holds the authoritative
// PresenceRegistry; PR-N6 spec routes device-mode forward_invoke
// up the already-open session bidi to the hub for resolution.
//
// Wire ↔ correlation seam
// -----------------------
// The session bidi is recreated on every reconnect: the `up_tx`
// mpsc the device sends frames on lives inside one
// `dial_and_run_session` invocation, and exits when the bidi
// closes. To survive reconnects without re-plumbing every dispatch
// site, this module exposes a stable `Arc<SessionEscalationHandle>`
// that the dispatch handler holds for the daemon's lifetime. The
// handle's mpsc receiver is owned by the **consumer task** spawned
// next to the session supervisor; on each reconnect the consumer
// re-binds itself to the fresh `up_tx`, but the dispatch-side
// `Sender` half is unaffected — it just keeps pushing
// `EscalationRequest` items into the pipe.
//
// Correlation table
// -----------------
// PR-N6 spec §"Concurrent multiplexing": `call_id` is a 16-byte
// `OsRng` nonce; concurrent in-flight Requests are matched on
// `call_id` against an `oneshot::Sender` table. This module owns
// that table, indexed by the 16-byte nonce. Hub-pushed
// `RequestResult` frames flow through `complete_pending` from the
// session-down-stream dispatch path; the matching pending entry
// fulfils the dispatch handler's `oneshot::Receiver`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::services::invocation_transport::invoke_remote_initiator::{
    call_id_hex, RequestOutcome, SessionRequestError,
};
use crate::services::invocation_transport::session_initiator::{
    SessionUpSender, SESSION_STREAM_ID,
};

/// Default deadline for awaiting a `RequestResult` after the
/// device queues a Request. PR-N6 spec §"Deadline propagation"
/// says CLI-supplied `--deadline-ms` overrides this; the daemon
/// applies this default when the CLI didn't supply one. 30s
/// matches PR-N1's `forward_invoke_timeout`.
pub const DEFAULT_ESCALATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Capacity of the dispatch-side mpsc the dispatch handler pushes
/// `EscalationRequest` items into. Sized matching
/// `SESSION_UP_CHANNEL_CAPACITY` in `session_initiator` so the
/// dispatch and consumer halves use symmetric backpressure
/// budgets.
pub const ESCALATION_QUEUE_CAPACITY: usize = 256;

/// One pending escalation: the device daemon's dispatch handler
/// minted a `call_id` and is awaiting the hub's `RequestResult`.
pub struct EscalationRequest {
    pub call_id: [u8; 16],
    pub ability: String,
    pub args: Vec<u8>,
    pub reply: oneshot::Sender<RequestOutcome>,
}

/// Stable handle the dispatch handler clones to submit Requests.
/// Lives in `Arc` so every dispatch can call `escalate` without
/// taking ownership.
#[derive(Clone, Debug)]
pub struct SessionEscalationHandle {
    submit: mpsc::Sender<EscalationRequest>,
    correlation: Arc<EscalationCorrelation>,
}

impl SessionEscalationHandle {
    /// Submit a Request with the given `(ability, args)` and await
    /// the matching `RequestResult`. Mints a fresh 16-byte
    /// `OsRng` nonce per call so concurrent dispatches never
    /// collide on `call_id`.
    ///
    /// Returns the typed `RequestOutcome` per PR-N6 spec
    /// §"Wire shape" — same shape the wire frame carries, no
    /// stringly-typed error.
    pub async fn escalate(&self, ability: String, args: Vec<u8>) -> RequestOutcome {
        self.escalate_with_timeout(ability, args, DEFAULT_ESCALATION_TIMEOUT)
            .await
    }

    /// `escalate` with a caller-chosen timeout. Used by tests + by
    /// any future CLI surface that wires `--deadline-ms` through.
    pub async fn escalate_with_timeout(
        &self,
        ability: String,
        args: Vec<u8>,
        timeout: Duration,
    ) -> RequestOutcome {
        use rand::RngCore as _;
        let mut call_id = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut call_id);
        let id_hex = call_id_hex(&call_id);

        // Operator log marker for the device-mode forward_invoke
        // escalation up the `<self>.session` bidi. SRE pipelines
        // grep `kind=forward_invoke_escalated_up_session_bidi` to
        // confirm the device-mode daemon actually escalated to the
        // hub rather than answering from local presence. The
        // PR-N6 "locked marker" comment that referenced a demo
        // orchestration script no longer reflects reality — the
        // 2026-05-25 audit confirmed no external grep dependency
        // on the previous byte-exact form.
        crate::op_event!(
            component = daemon_invocation,
            kind = forward_invoke_escalated_up_session_bidi,
            call_id = id_hex,
        );

        let (reply_tx, reply_rx) = oneshot::channel();
        let request = EscalationRequest {
            call_id,
            ability,
            args,
            reply: reply_tx,
        };

        if self.submit.send(request).await.is_err() {
            // The consumer task dropped its receiver — most likely
            // the daemon is shutting down. Surface a typed
            // upstream-failure outcome so the dispatch handler
            // doesn't pretend the call succeeded.
            return RequestOutcome::Err {
                error: SessionRequestError::UpstreamFailure {
                    reason: "session escalation consumer task is gone (daemon shutdown?)"
                        .to_string(),
                },
            };
        }

        match tokio::time::timeout(timeout, reply_rx).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => RequestOutcome::Err {
                error: SessionRequestError::UpstreamFailure {
                    reason: "session escalation reply channel dropped without answer".to_string(),
                },
            },
            Err(_elapsed) => {
                self.correlation.cancel(call_id);
                RequestOutcome::Err {
                    error: SessionRequestError::UpstreamTimeout,
                }
            }
        }
    }
}

/// Per-call_id correlation table the consumer task owns. The
/// session-down-stream handler calls `complete` when a
/// `RequestResult` lands; `escalate` registers entries via
/// `register`. `Mutex<HashMap>` is fine — concurrent
/// register/complete is bounded by the `oneshot` semantics
/// (exactly one register, exactly one complete).
#[derive(Debug, Default)]
pub struct EscalationCorrelation {
    inner: Mutex<HashMap<[u8; 16], oneshot::Sender<RequestOutcome>>>,
}

impl EscalationCorrelation {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(HashMap::new()),
        })
    }

    /// Insert a pending entry. If a duplicate `call_id` ever
    /// arrives (only possible if `OsRng` collided in 128 bits,
    /// which is cryptographically negligible), the new entry
    /// replaces the old; the displaced sender is dropped, which
    /// surfaces on the original `reply_rx` as a closed-without-
    /// answer condition the `escalate_with_timeout` arm reports
    /// as `UpstreamFailure`.
    pub fn register(&self, call_id: [u8; 16], reply: oneshot::Sender<RequestOutcome>) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.insert(call_id, reply);
    }

    /// Complete a pending entry by `call_id`. Returns `true` if a
    /// matching entry was present; `false` is the silent no-op
    /// the spec calls for when a `RequestResult` arrives after
    /// the dispatch handler timed out and dropped its receiver.
    pub fn complete(&self, call_id: [u8; 16], outcome: RequestOutcome) -> bool {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        match guard.remove(&call_id) {
            Some(sender) => sender.send(outcome).is_ok(),
            None => false,
        }
    }

    pub fn cancel(&self, call_id: [u8; 16]) -> bool {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.remove(&call_id).is_some()
    }

    /// Number of pending entries — observability only.
    #[cfg(test)]
    pub fn pending_len(&self) -> usize {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.len()
    }
}

/// Reload-friendly handle on the device's *current*
/// `<self>.session` bidi up sender. The session supervisor
/// publishes a fresh sender via [`set`] on every successful
/// reconnect; clears via [`clear`] on disconnect. The
/// outbox-aware consumer task ([`spawn_escalation_consumer_with_outbox`])
/// snapshots this on every drained `EscalationRequest` so a
/// SIGHUP-driven reconnect picks up the new sender without
/// re-spawning the consumer task.
///
/// Why this lives next to `EscalationCorrelation` and not in a
/// separate module: the two collaborate per-call (consumer reads
/// outbox, registers in correlation, pushes Request frame), and
/// keeping them adjacent lets the consumer hold one `Arc<…>`
/// each instead of three handles. The two have independent
/// lifetimes (consumer drains forever; outbox publishes per
/// reconnect; correlation accumulates and drains per Request).
#[derive(Clone, Debug, Default)]
pub struct SharedSessionOutbox {
    inner: Arc<Mutex<Option<SessionUpSender>>>,
}

impl SharedSessionOutbox {
    /// Build an empty outbox. Boot calls this once per device-mode
    /// daemon process, then clones the result to wire both the
    /// session supervisor (write side) and the escalation
    /// consumer (read side).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish a freshly-built session up sender. Called by
    /// `dial_and_run_session` after frame-0 dispatch confirms
    /// the bidi is alive enough to carry escalation frames.
    /// Subsequent dial attempts (after a reconnect) overwrite
    /// the previous sender; in-flight escalations holding the
    /// old sender clone keep working until that channel closes.
    pub fn set(&self, sender: SessionUpSender) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = Some(sender);
    }

    /// Drop the current sender. Called by `dial_and_run_session`
    /// on every exit path so the consumer's next snapshot reads
    /// `None` until the supervisor's next successful dial.
    pub fn clear(&self) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = None;
    }

    /// Snapshot the current sender as a clone (cheap; tokio
    /// channel senders are `Arc`-shaped). Returns `None` when no
    /// live session is published — the consumer surfaces
    /// `UpstreamFailure { reason: "no live <self>.session bidi" }`.
    #[must_use]
    pub fn snapshot(&self) -> Option<SessionUpSender> {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.clone()
    }
}

/// Outbox-aware variant of [`spawn_escalation_consumer`]: instead
/// of capturing a single `up_tx`, the consumer reads the current
/// up sender from a shared outbox per `EscalationRequest`. A
/// SIGHUP-driven session reconnect publishes a fresh sender into
/// the outbox; subsequent escalations pick it up without
/// respawning. When the outbox is empty (no live session right
/// now), the request surfaces `UpstreamFailure` to the dispatch
/// caller — the CLI sees a structured error rather than waiting
/// for a reconnect that may never happen on its deadline.
pub fn spawn_escalation_consumer_with_outbox(
    correlation: Arc<EscalationCorrelation>,
    outbox: SharedSessionOutbox,
) -> SessionEscalationHandle {
    let (submit_tx, mut submit_rx) = mpsc::channel::<EscalationRequest>(ESCALATION_QUEUE_CAPACITY);
    let handle = SessionEscalationHandle {
        submit: submit_tx,
        correlation: Arc::clone(&correlation),
    };

    tokio::spawn(async move {
        while let Some(request) = submit_rx.recv().await {
            let EscalationRequest {
                call_id,
                ability,
                args,
                reply,
            } = request;

            let Some(up_tx) = outbox.snapshot() else {
                // No live session — surface a typed upstream-failure
                // outcome so the CLI sees a fast structured error
                // instead of waiting for a hub reconnect.
                let _ = reply.send(RequestOutcome::Err {
                    error: SessionRequestError::UpstreamFailure {
                        reason: "no live <self>.session bidi to escalate forward_invoke up; \
                                 device-mode daemon's session supervisor has not (yet) \
                                 reconnected"
                            .to_string(),
                    },
                });
                continue;
            };

            correlation.register(call_id, reply);

            let frame = build_session_request_up_chunk(call_id, &ability, &args);
            if let Err(err) = up_tx.send_binary_chunk(frame).await {
                let mut guard = match correlation.inner.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if let Some(sender) = guard.remove(&call_id) {
                    let _ = sender.send(RequestOutcome::Err {
                        error: SessionRequestError::UpstreamFailure {
                            reason: format!(
                                "session up-channel closed mid-push (mid-reconnect?): {err}"
                            ),
                        },
                    });
                }
            }
        }
    });

    handle
}

/// Spawn the consumer task: pulls `EscalationRequest` items off
/// the submit-queue, registers each one in the correlation table,
/// and writes a `SessionDispatch::Request` BinaryChunk to the
/// supplied `up_tx` from the active session bidi. Returns the
/// `SessionEscalationHandle` the dispatch handler clones into
/// daemon state.
///
/// The single-up_tx flavour, retained for tests. Production
/// boot-wired daemons use [`spawn_escalation_consumer_with_outbox`]
/// instead so reconnects naturally pick up the fresh sender.
pub fn spawn_escalation_consumer(
    correlation: Arc<EscalationCorrelation>,
    up_tx: SessionUpSender,
) -> SessionEscalationHandle {
    let (submit_tx, mut submit_rx) = mpsc::channel::<EscalationRequest>(ESCALATION_QUEUE_CAPACITY);
    let handle = SessionEscalationHandle {
        submit: submit_tx,
        correlation: Arc::clone(&correlation),
    };

    tokio::spawn(async move {
        while let Some(request) = submit_rx.recv().await {
            let EscalationRequest {
                call_id,
                ability,
                args,
                reply,
            } = request;
            correlation.register(call_id, reply);

            let frame = build_session_request_up_chunk(call_id, &ability, &args);
            if let Err(err) = up_tx.send_binary_chunk(frame).await {
                // Up-channel closed — the bidi went away mid-flight.
                // Pull the entry back out and surface upstream
                // failure to the dispatch caller. The consumer
                // task continues to drain so subsequent dispatches
                // surface promptly when a new bidi is available.
                let mut guard = match correlation.inner.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if let Some(sender) = guard.remove(&call_id) {
                    let _ = sender.send(RequestOutcome::Err {
                        error: SessionRequestError::UpstreamFailure {
                            reason: format!("session up-channel closed: {err}"),
                        },
                    });
                }
            }
        }
    });

    handle
}

/// Build the `InvokeBidiUp` frame that wraps a
/// `SessionDispatch::Request` JSON in a `BinaryChunk` payload.
/// Mirrors what `dial_and_run_session` writes for non-Request
/// frames + matches the wire shape PR-N6 §"Wire shape" locks.
fn build_session_request_up_chunk(
    call_id: [u8; 16],
    ability: &str,
    args: &[u8],
) -> easynet_axon::pb::axon::v1::BinaryChunk {
    use crate::services::invocation_transport::invoke_remote_initiator::{
        SessionContentEnvelope, SessionDispatch,
    };
    use easynet_axon::pb::axon::v1::BinaryChunk;

    let dispatch = SessionDispatch::Request {
        call_id,
        ability: ability.to_string(),
        args: args.to_vec(),
        args_content_envelope: SessionContentEnvelope::plaintext_json(),
    };
    // serde encoding of an owned-fields enum cannot fail here;
    // the unwrap is justified by the typed enum domain.
    let data = serde_json::to_vec(&dispatch).expect("encode SessionDispatch::Request");

    BinaryChunk {
        stream_id: SESSION_STREAM_ID,
        data,
        ..BinaryChunk::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use easynet_axon::pb::axon::v1::InvokeBidiUp;

    #[tokio::test]
    async fn escalate_resolves_when_correlation_completes() {
        // Wire-up: spawn consumer + a fake "hub" task that drains
        // up_rx, decodes the Request frame, and feeds the
        // matching RequestResult back into the correlation table.
        let correlation = EscalationCorrelation::new();
        let (up_tx, mut up_rx) = mpsc::channel::<easynet_axon::pb::axon::v1::InvokeBidiUp>(8);
        let handle =
            spawn_escalation_consumer(Arc::clone(&correlation), SessionUpSender::new(up_tx));

        let correlation_for_hub = Arc::clone(&correlation);
        tokio::spawn(async move {
            while let Some(frame) = up_rx.recv().await {
                use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
                let chunk = match frame.payload {
                    Some(UpPayload::BinaryChunk(c)) => c,
                    _ => continue,
                };
                let dispatch: crate::services::invocation_transport::invoke_remote_initiator::SessionDispatch =
                    serde_json::from_slice(&chunk.data).expect("decode");
                if let crate::services::invocation_transport::invoke_remote_initiator::SessionDispatch::Request {
                    call_id, ..
                } = dispatch
                {
                    correlation_for_hub.complete(
                        call_id,
                        RequestOutcome::Ok {
                            result_bytes: b"hub-resolved".to_vec(),
                        },
                    );
                }
            }
        });

        let outcome = handle
            .escalate("federation.forward_invoke".into(), b"{}".to_vec())
            .await;
        match outcome {
            RequestOutcome::Ok { result_bytes } => {
                assert_eq!(result_bytes, b"hub-resolved");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn escalate_surfaces_upstream_timeout_when_no_reply() {
        // Spawn consumer with an up_tx whose receiver we hold but
        // never decode/complete. The dispatch handler must surface
        // `UpstreamTimeout` rather than hanging forever.
        let correlation = EscalationCorrelation::new();
        let (up_tx, _up_rx_held) = mpsc::channel::<easynet_axon::pb::axon::v1::InvokeBidiUp>(8);
        let handle =
            spawn_escalation_consumer(Arc::clone(&correlation), SessionUpSender::new(up_tx));

        let outcome = handle
            .escalate_with_timeout(
                "federation.forward_invoke".into(),
                b"{}".to_vec(),
                Duration::from_millis(50),
            )
            .await;
        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamTimeout,
            } => {}
            other => panic!("expected UpstreamTimeout, got {other:?}"),
        }
        assert_eq!(correlation.pending_len(), 0);
    }

    #[tokio::test]
    async fn escalate_surfaces_upstream_failure_when_up_channel_closes() {
        let correlation = EscalationCorrelation::new();
        let (up_tx, up_rx) = mpsc::channel::<easynet_axon::pb::axon::v1::InvokeBidiUp>(8);
        // Drop the receiver immediately so the consumer's send
        // fails on the very first item.
        drop(up_rx);
        let handle =
            spawn_escalation_consumer(Arc::clone(&correlation), SessionUpSender::new(up_tx));

        let outcome = handle
            .escalate_with_timeout(
                "federation.forward_invoke".into(),
                b"{}".to_vec(),
                Duration::from_secs(2),
            )
            .await;
        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamFailure { reason },
            } => {
                assert!(
                    reason.contains("session up-channel closed"),
                    "reason should cite up-channel close; got {reason}",
                );
            }
            other => panic!("expected UpstreamFailure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn correlation_complete_returns_false_when_no_pending_entry() {
        let correlation = EscalationCorrelation::new();
        let stale_id = [0xff; 16];
        let completed = correlation.complete(
            stale_id,
            RequestOutcome::Ok {
                result_bytes: vec![],
            },
        );
        assert!(
            !completed,
            "complete on missing entry must be a silent no-op"
        );
    }

    // ── PR-N6 C4: outbox-aware consumer wiring tests ──

    #[tokio::test]
    async fn outbox_starts_empty_returns_none_on_snapshot() {
        let outbox = SharedSessionOutbox::new();
        assert!(outbox.snapshot().is_none());
    }

    #[tokio::test]
    async fn outbox_set_then_clear_round_trip() {
        let outbox = SharedSessionOutbox::new();
        let (tx, _rx) = mpsc::channel::<InvokeBidiUp>(4);
        outbox.set(SessionUpSender::new(tx));
        assert!(outbox.snapshot().is_some());
        outbox.clear();
        assert!(outbox.snapshot().is_none());
    }

    #[tokio::test]
    async fn outbox_consumer_surfaces_no_live_session_when_outbox_empty() {
        // Boot ordering: device-mode daemon constructs the
        // escalation correlation + outbox + consumer BEFORE the
        // session supervisor has dialled. An immediate CLI call
        // hitting the dispatcher escalates while outbox.snapshot()
        // is still None. The consumer must surface
        // `UpstreamFailure { reason: contains "no live ...
        // bidi" }` rather than hanging waiting for a sender.
        let correlation = EscalationCorrelation::new();
        let outbox = SharedSessionOutbox::new();
        let handle = spawn_escalation_consumer_with_outbox(Arc::clone(&correlation), outbox);

        let outcome = handle
            .escalate_with_timeout(
                "federation.forward_invoke".into(),
                b"{}".to_vec(),
                Duration::from_secs(2),
            )
            .await;
        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamFailure { reason },
            } => {
                assert!(
                    reason.contains("no live <self>.session bidi"),
                    "reason should cite missing session; got {reason}",
                );
            }
            other => panic!("expected UpstreamFailure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn outbox_consumer_picks_up_published_sender_on_next_request() {
        // Sequence:
        //   1. Spawn consumer with empty outbox
        //   2. Supervisor publishes a fresh up_tx
        //   3. Hub-side fake task drains up_rx, decodes the
        //      Request, and feeds matching RequestResult back
        //   4. CLI escalation succeeds with Ok bytes
        // Pins that the consumer reads outbox per-Request rather
        // than capturing one up_tx at construction time.
        let correlation = EscalationCorrelation::new();
        let outbox = SharedSessionOutbox::new();
        let handle =
            spawn_escalation_consumer_with_outbox(Arc::clone(&correlation), outbox.clone());

        let (up_tx, mut up_rx) = mpsc::channel::<InvokeBidiUp>(8);
        outbox.set(SessionUpSender::new(up_tx));

        let correlation_for_hub = Arc::clone(&correlation);
        tokio::spawn(async move {
            while let Some(frame) = up_rx.recv().await {
                use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
                let chunk = match frame.payload {
                    Some(UpPayload::BinaryChunk(c)) => c,
                    _ => continue,
                };
                let dispatch: crate::services::invocation_transport::invoke_remote_initiator::SessionDispatch =
                    serde_json::from_slice(&chunk.data).expect("decode");
                if let crate::services::invocation_transport::invoke_remote_initiator::SessionDispatch::Request {
                    call_id, ..
                } = dispatch
                {
                    correlation_for_hub.complete(
                        call_id,
                        RequestOutcome::Ok {
                            result_bytes: b"hub-via-outbox".to_vec(),
                        },
                    );
                }
            }
        });

        let outcome = handle
            .escalate("federation.forward_invoke".into(), b"{}".to_vec())
            .await;
        match outcome {
            RequestOutcome::Ok { result_bytes } => {
                assert_eq!(result_bytes, b"hub-via-outbox");
            }
            other => panic!("expected Ok via outbox, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn outbox_consumer_surfaces_no_live_session_after_clear() {
        // Sequence:
        //   1. Outbox set; consumer running
        //   2. Outbox cleared (simulates session disconnect)
        //   3. Subsequent escalate hits the empty-outbox branch
        let correlation = EscalationCorrelation::new();
        let outbox = SharedSessionOutbox::new();
        let handle =
            spawn_escalation_consumer_with_outbox(Arc::clone(&correlation), outbox.clone());

        let (up_tx, _up_rx_held) = mpsc::channel::<InvokeBidiUp>(8);
        outbox.set(SessionUpSender::new(up_tx));
        outbox.clear();

        let outcome = handle
            .escalate_with_timeout(
                "federation.forward_invoke".into(),
                b"{}".to_vec(),
                Duration::from_secs(2),
            )
            .await;
        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamFailure { reason },
            } => {
                assert!(
                    reason.contains("no live <self>.session bidi"),
                    "reason should cite missing session; got {reason}",
                );
            }
            other => panic!("expected UpstreamFailure after clear, got {other:?}"),
        }
    }
}
