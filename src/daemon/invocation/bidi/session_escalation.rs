// EasyNet CLI — invocation_transport — reverse-session invocation relay
// ======================================================================
//
// File: src/daemon/invocation/session_escalation.rs
// Description: Relays complete signed InvokeRequests from a device daemon to
//              its hub as typed `ReverseDispatchCall` frames. Daemon-owned
//              bootstrap control calls retain a separate bounded Request /
//              RequestResult codec.
//
// Why this module exists
// ----------------------
// Device-mode daemons only dial outbound `session.open` and never
// accept inbound bidi; their local `PresenceRegistry` is empty by
// construction (no peer ever calls `presence.insert` on a
// device-mode daemon's registry). The hub holds the authoritative
// PresenceRegistry, so canonical remote calls travel up the already-open
// session to the hub for routing.
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

use crate::daemon::invocation::bidi::session_initiator::{SessionUpSender, SESSION_STREAM_ID};
use crate::daemon::invocation::bidi::session_wire::{
    call_id_hex, RequestOutcome, SessionRequestError,
};
use crate::daemon::invocation::bidi::state::pending_dispatch::{
    DispatchResult, DispatchStreamEvent,
};
use crate::daemon::invocation::bidi::state::session_failure::SessionFailure;

/// Default deadline for awaiting a `RequestResult` after the
/// device queues a Request. PR-N6 spec §"Deadline propagation"
/// says CLI-supplied `--deadline-ms` overrides this; the daemon
/// applies this default when the CLI didn't supply one. 30s
/// matches the canonical remote invocation deadline.
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
    pub invocation: EscalationInvocation,
    pub reply: EscalationReplySink,
}

pub enum EscalationCommand {
    Request(EscalationRequest),
    BidiInput(EscalationBidiInput),
}

pub struct EscalationBidiInput {
    pub call_id: [u8; 16],
    pub input: EscalatedBidiInput,
}

pub enum EscalatedBidiInput {
    Binary(axon_sdk::pb::axon::v1::BinaryChunk),
    Control(axon_sdk::pb::axon::v1::BidiControl),
}

/// Payload carried by a reverse session request.
///
/// Product invocations use `Canonical` exclusively. `DaemonControl` is a
/// separate control-plane protocol used only for bootstrap, publication, and
/// trust synchronization; it is not an Invocation compatibility path.
pub enum EscalationInvocation {
    Canonical(Box<axon_sdk::pb::axon::v1::InvokeRequest>),
    CanonicalStream(Box<axon_sdk::pb::axon::v1::InvokeServerStreamRequest>),
    CanonicalBidi(Box<axon_sdk::pb::axon::v1::InvokeRequest>),
    DaemonControl { ability_ura: String, args: Vec<u8> },
}

/// Reply channel registered for one session-escalated request. Unary product
/// calls and daemon-control calls complete with one result; server-stream calls
/// register a bounded event channel and receive many reverse-dispatch result
/// frames until the terminal checkpoint.
pub enum EscalationReplySink {
    Unary(oneshot::Sender<EscalationReply>),
    Stream {
        events: mpsc::Sender<DispatchStreamEvent>,
        accepted: oneshot::Sender<Result<(), SessionRequestError>>,
    },
}

/// Internal correlation result. Canonical product invocations retain the
/// complete InvokeResponse (including both signed finalization checkpoints),
/// while daemon-owned control requests keep their separate JSON outcome.
#[derive(Debug, Clone)]
pub enum EscalationReply {
    Canonical(Box<axon_sdk::pb::axon::v1::InvokeResponse>),
    Control(RequestOutcome),
    Error(SessionRequestError),
}

/// Stream handle returned to the device-mode `InvokeStream` dispatcher after
/// the request has been written to the live `session.open` bidi. Dropping the
/// handle removes the correlation entry, so a cancelled client cannot leak a
/// reverse-dispatch stream in the device daemon.
pub struct EscalatedStreamHandle {
    call_id: [u8; 16],
    correlation: Arc<EscalationCorrelation>,
    rx: mpsc::Receiver<DispatchStreamEvent>,
}

impl EscalatedStreamHandle {
    #[must_use]
    pub fn call_id(&self) -> [u8; 16] {
        self.call_id
    }

    pub async fn recv(&mut self) -> Option<DispatchStreamEvent> {
        self.rx.recv().await
    }
}

impl Drop for EscalatedStreamHandle {
    fn drop(&mut self) {
        self.correlation.cancel(self.call_id);
    }
}

/// Bidi handle returned to a device-mode dispatcher after the reverse-bidi
/// open frame is queued on the live session. Output is correlated by the same
/// 16-byte reverse call nonce as unary/stream escalation; input frames are
/// serialized through the escalation consumer so open/input ordering is
/// preserved on reconnect-bound session outboxes.
pub struct EscalatedBidiHandle {
    call_id: [u8; 16],
    submit: mpsc::Sender<EscalationCommand>,
    correlation: Arc<EscalationCorrelation>,
    rx: mpsc::Receiver<DispatchStreamEvent>,
}

impl EscalatedBidiHandle {
    #[must_use]
    pub fn call_id(&self) -> [u8; 16] {
        self.call_id
    }

    pub async fn recv(&mut self) -> Option<DispatchStreamEvent> {
        self.rx.recv().await
    }

    #[must_use]
    pub fn input_sender(&self) -> EscalatedBidiInputSender {
        EscalatedBidiInputSender {
            call_id: self.call_id,
            submit: self.submit.clone(),
        }
    }

    pub async fn send_binary(
        &self,
        chunk: axon_sdk::pb::axon::v1::BinaryChunk,
    ) -> Result<(), SessionRequestError> {
        self.send_input(EscalatedBidiInput::Binary(chunk)).await
    }

    pub async fn send_control(
        &self,
        control: axon_sdk::pb::axon::v1::BidiControl,
    ) -> Result<(), SessionRequestError> {
        self.send_input(EscalatedBidiInput::Control(control)).await
    }

    async fn send_input(&self, input: EscalatedBidiInput) -> Result<(), SessionRequestError> {
        self.submit
            .send(EscalationCommand::BidiInput(EscalationBidiInput {
                call_id: self.call_id,
                input,
            }))
            .await
            .map_err(|_| SessionRequestError::UpstreamFailure {
                reason: "session escalation consumer task is gone (daemon shutdown?)".to_string(),
            })
    }
}

impl Drop for EscalatedBidiHandle {
    fn drop(&mut self) {
        self.correlation.cancel(self.call_id);
    }
}

#[derive(Clone)]
pub struct EscalatedBidiInputSender {
    call_id: [u8; 16],
    submit: mpsc::Sender<EscalationCommand>,
}

impl EscalatedBidiInputSender {
    pub async fn send_binary(
        &self,
        chunk: axon_sdk::pb::axon::v1::BinaryChunk,
    ) -> Result<(), SessionRequestError> {
        self.send_input(EscalatedBidiInput::Binary(chunk)).await
    }

    pub async fn send_control(
        &self,
        control: axon_sdk::pb::axon::v1::BidiControl,
    ) -> Result<(), SessionRequestError> {
        self.send_input(EscalatedBidiInput::Control(control)).await
    }

    async fn send_input(&self, input: EscalatedBidiInput) -> Result<(), SessionRequestError> {
        self.submit
            .send(EscalationCommand::BidiInput(EscalationBidiInput {
                call_id: self.call_id,
                input,
            }))
            .await
            .map_err(|_| SessionRequestError::UpstreamFailure {
                reason: "session escalation consumer task is gone (daemon shutdown?)".to_string(),
            })
    }
}

/// Stable handle the dispatch handler clones to submit Requests.
/// Lives in `Arc` so every dispatch can call `escalate` without
/// taking ownership.
#[derive(Clone, Debug)]
pub struct SessionEscalationHandle {
    submit: mpsc::Sender<EscalationCommand>,
    correlation: Arc<EscalationCorrelation>,
    session_realm: String,
}

impl SessionEscalationHandle {
    /// Relay one already-signed canonical invocation through the device's
    /// daemon-owned session without projecting or rebuilding any tuple field.
    pub async fn escalate_invoke(
        &self,
        request: axon_sdk::pb::axon::v1::InvokeRequest,
    ) -> Result<axon_sdk::pb::axon::v1::InvokeResponse, SessionRequestError> {
        match self
            .escalate_invocation(
                EscalationInvocation::Canonical(Box::new(request)),
                DEFAULT_ESCALATION_TIMEOUT,
            )
            .await
        {
            EscalationReply::Canonical(response) => Ok(*response),
            EscalationReply::Error(error) => Err(error),
            EscalationReply::Control(_) => Err(SessionRequestError::UpstreamFailure {
                reason: "canonical escalation received a daemon-control reply".to_string(),
            }),
        }
    }

    /// Relay one already-signed canonical server-stream invocation through the
    /// device-owned session. The hub remains the route/admission authority; the
    /// device only correlates the reverse-dispatch result stream by call nonce.
    pub async fn escalate_stream(
        &self,
        request: axon_sdk::pb::axon::v1::InvokeServerStreamRequest,
    ) -> Result<EscalatedStreamHandle, SessionRequestError> {
        use rand::RngCore as _;
        let mut call_id = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut call_id);
        let id_hex = call_id_hex(&call_id);

        crate::op_event!(
            component = daemon_invocation,
            kind = canonical_invoke_stream_escalated_up_session_bidi,
            call_id = id_hex,
        );

        let (events_tx, events_rx) = mpsc::channel(32);
        let (accepted_tx, accepted_rx) = oneshot::channel();
        let request = EscalationRequest {
            call_id,
            invocation: EscalationInvocation::CanonicalStream(Box::new(request)),
            reply: EscalationReplySink::Stream {
                events: events_tx,
                accepted: accepted_tx,
            },
        };
        if self
            .submit
            .send(EscalationCommand::Request(request))
            .await
            .is_err()
        {
            return Err(SessionRequestError::UpstreamFailure {
                reason: "session escalation consumer task is gone (daemon shutdown?)".to_string(),
            });
        }
        match tokio::time::timeout(DEFAULT_ESCALATION_TIMEOUT, accepted_rx).await {
            Ok(Ok(Ok(()))) => Ok(EscalatedStreamHandle {
                call_id,
                correlation: Arc::clone(&self.correlation),
                rx: events_rx,
            }),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(_)) => Err(SessionRequestError::UpstreamFailure {
                reason: "session stream escalation acknowledgement channel dropped".to_string(),
            }),
            Err(_) => {
                self.correlation.cancel(call_id);
                Err(SessionRequestError::UpstreamTimeout)
            }
        }
    }

    /// Relay one already-signed canonical bidi invocation through the
    /// device-owned session. This is the reverse-channel equivalent of
    /// `InvokeBidi`: open is a `ReverseDispatchCall(open_bidi=true)`, input is
    /// `ReverseBidiInput`, and output returns as `ReverseDispatchResult`.
    pub async fn escalate_bidi(
        &self,
        request: axon_sdk::pb::axon::v1::InvokeRequest,
    ) -> Result<EscalatedBidiHandle, SessionRequestError> {
        use rand::RngCore as _;
        let mut call_id = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut call_id);
        let id_hex = call_id_hex(&call_id);

        crate::op_event!(
            component = daemon_invocation,
            kind = canonical_invoke_bidi_escalated_up_session_bidi,
            call_id = id_hex,
        );

        let (events_tx, events_rx) = mpsc::channel(32);
        let (accepted_tx, accepted_rx) = oneshot::channel();
        let request = EscalationRequest {
            call_id,
            invocation: EscalationInvocation::CanonicalBidi(Box::new(request)),
            reply: EscalationReplySink::Stream {
                events: events_tx,
                accepted: accepted_tx,
            },
        };
        if self
            .submit
            .send(EscalationCommand::Request(request))
            .await
            .is_err()
        {
            return Err(SessionRequestError::UpstreamFailure {
                reason: "session escalation consumer task is gone (daemon shutdown?)".to_string(),
            });
        }
        match tokio::time::timeout(DEFAULT_ESCALATION_TIMEOUT, accepted_rx).await {
            Ok(Ok(Ok(()))) => Ok(EscalatedBidiHandle {
                call_id,
                submit: self.submit.clone(),
                correlation: Arc::clone(&self.correlation),
                rx: events_rx,
            }),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(_)) => Err(SessionRequestError::UpstreamFailure {
                reason: "session bidi escalation acknowledgement channel dropped".to_string(),
            }),
            Err(_) => {
                self.correlation.cancel(call_id);
                Err(SessionRequestError::UpstreamTimeout)
            }
        }
    }

    /// Submit a Request for a hub-owned public wrapper ability and
    /// await the matching `RequestResult`. The wire frame carries
    /// the derived `ability_ura`, not this internal public-name
    /// parameter. Mints a fresh 16-byte `OsRng` nonce per call so
    /// concurrent dispatches never collide on `call_id`.
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
        let ability_ura = crate::core::ura::hub_ability_ura(&self.session_realm, ability.trim());
        match self
            .escalate_invocation(
                EscalationInvocation::DaemonControl { ability_ura, args },
                timeout,
            )
            .await
        {
            EscalationReply::Control(outcome) => outcome,
            EscalationReply::Error(error) => RequestOutcome::Err { error },
            EscalationReply::Canonical(_) => RequestOutcome::Err {
                error: SessionRequestError::UpstreamFailure {
                    reason: "daemon-control escalation received a canonical Invocation reply"
                        .to_string(),
                },
            },
        }
    }

    async fn escalate_invocation(
        &self,
        invocation: EscalationInvocation,
        timeout: Duration,
    ) -> EscalationReply {
        use rand::RngCore as _;
        let mut call_id = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut call_id);
        let id_hex = call_id_hex(&call_id);

        // Operator log marker for the device-mode canonical_invoke
        // escalation up the `session.open` bidi. SRE pipelines
        // grep `kind=canonical_invoke_escalated_up_session_bidi` to
        // confirm the device-mode daemon actually escalated to the
        // hub rather than answering from local presence. The
        // PR-N6 "locked marker" comment that referenced a demo
        // orchestration script no longer reflects reality — the
        // 2026-05-25 audit confirmed no external grep dependency
        // on the previous byte-exact form.
        crate::op_event!(
            component = daemon_invocation,
            kind = canonical_invoke_escalated_up_session_bidi,
            call_id = id_hex,
        );

        let (reply_tx, reply_rx) = oneshot::channel();
        let request = EscalationRequest {
            call_id,
            invocation,
            reply: EscalationReplySink::Unary(reply_tx),
        };

        if self
            .submit
            .send(EscalationCommand::Request(request))
            .await
            .is_err()
        {
            // The consumer task dropped its receiver — most likely
            // the daemon is shutting down. Surface a typed
            // upstream-failure outcome so the dispatch handler
            // doesn't pretend the call succeeded.
            return EscalationReply::Error(SessionRequestError::UpstreamFailure {
                reason: "session escalation consumer task is gone (daemon shutdown?)".to_string(),
            });
        }

        match tokio::time::timeout(timeout, reply_rx).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => EscalationReply::Error(SessionRequestError::UpstreamFailure {
                reason: "session escalation reply channel dropped without answer".to_string(),
            }),
            Err(_elapsed) => {
                self.correlation.cancel(call_id);
                EscalationReply::Error(SessionRequestError::UpstreamTimeout)
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
    inner: Mutex<HashMap<[u8; 16], PendingEscalation>>,
}

#[derive(Debug)]
enum PendingEscalation {
    Unary(oneshot::Sender<EscalationReply>),
    Stream(mpsc::Sender<DispatchStreamEvent>),
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
    pub fn register_unary(&self, call_id: [u8; 16], reply: oneshot::Sender<EscalationReply>) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.insert(call_id, PendingEscalation::Unary(reply));
    }

    pub fn register_stream(&self, call_id: [u8; 16], sender: mpsc::Sender<DispatchStreamEvent>) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.insert(call_id, PendingEscalation::Stream(sender));
    }

    /// Complete a pending entry by `call_id`. Returns `true` if a
    /// matching entry was present; `false` is the silent no-op
    /// the spec calls for when a `RequestResult` arrives after
    /// the dispatch handler timed out and dropped its receiver.
    pub fn complete(&self, call_id: [u8; 16], outcome: EscalationReply) -> bool {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        match guard.remove(&call_id) {
            Some(PendingEscalation::Unary(sender)) => sender.send(outcome).is_ok(),
            Some(PendingEscalation::Stream(sender)) => {
                let _ = sender.try_send(DispatchStreamEvent::Terminal(Box::new(
                    dispatch_result_from_escalation_reply(outcome),
                )));
                false
            }
            None => false,
        }
    }

    /// Deliver one protobuf reverse-dispatch result frame. Unary reverse
    /// dispatch must be terminal and complete a one-shot waiter. Stream reverse
    /// dispatch follows the same checkpoint geometry as normal carrier-v1
    /// streams: admission-only, zero or more data chunks, then terminal.
    pub fn deliver_reverse_dispatch_result(
        &self,
        call_id: [u8; 16],
        result: axon_sdk::pb::axon::v1::ReverseDispatchResult,
    ) -> bool {
        let pending = {
            let guard = match self.inner.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.get(&call_id).map(|entry| match entry {
                PendingEscalation::Unary(_) => PendingEscalationKind::Unary,
                PendingEscalation::Stream(sender) => PendingEscalationKind::Stream(sender.clone()),
            })
        };
        match pending {
            Some(PendingEscalationKind::Unary) => {
                if !result.terminal {
                    let mut guard = match self.inner.lock() {
                        Ok(g) => g,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    let Some(PendingEscalation::Unary(sender)) = guard.remove(&call_id) else {
                        return false;
                    };
                    return sender
                        .send(EscalationReply::Error(
                            SessionRequestError::UpstreamFailure {
                                reason:
                                    "unary reverse dispatch received a non-terminal stream frame"
                                        .to_string(),
                            },
                        ))
                        .is_ok();
                }
                let outcome = reverse_unary_reply(result);
                let mut guard = match self.inner.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                match guard.remove(&call_id) {
                    Some(PendingEscalation::Unary(sender)) => sender.send(outcome).is_ok(),
                    _ => false,
                }
            }
            Some(PendingEscalationKind::Stream(sender)) => {
                let (event, terminal) = reverse_stream_event(result);
                let delivered = match sender.try_send(event) {
                    Ok(()) => true,
                    Err(mpsc::error::TrySendError::Full(_))
                    | Err(mpsc::error::TrySendError::Closed(_)) => false,
                };
                if terminal || !delivered {
                    let mut guard = match self.inner.lock() {
                        Ok(g) => g,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    guard.remove(&call_id);
                }
                delivered
            }
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

enum PendingEscalationKind {
    Unary,
    Stream(mpsc::Sender<DispatchStreamEvent>),
}

fn reverse_unary_reply(result: axon_sdk::pb::axon::v1::ReverseDispatchResult) -> EscalationReply {
    match result.failure {
        None => {
            let result_content_type = result.result_content_type.trim().to_string();
            let state = result
                .terminal_receipt
                .as_ref()
                .map(|receipt| receipt.state)
                .unwrap_or(axon_sdk::pb::axon::v1::InvocationState::Completed as i32);
            EscalationReply::Canonical(Box::new(axon_sdk::pb::axon::v1::InvokeResponse {
                state,
                result: result.payload,
                result_content_type,
                admission_receipt: result.admission_receipt,
                terminal_receipt: result.terminal_receipt,
                ..axon_sdk::pb::axon::v1::InvokeResponse::default()
            }))
        }
        Some(failure) => EscalationReply::Error(session_request_error_from_wire_failure(failure)),
    }
}

fn reverse_stream_event(
    result: axon_sdk::pb::axon::v1::ReverseDispatchResult,
) -> (DispatchStreamEvent, bool) {
    if result.terminal {
        return (
            DispatchStreamEvent::Terminal(Box::new(dispatch_result_from_reverse_result(result))),
            true,
        );
    }
    if let Some(receipt) = result.admission_receipt {
        if result.terminal_receipt.is_some()
            || result.failure.is_some()
            || !result.payload.is_empty()
        {
            return (
                DispatchStreamEvent::Terminal(Box::new(DispatchResult {
                    payload: Vec::new(),
                    result_content_type: String::new(),
                    error: Some("malformed stream admission reverse dispatch result".to_string()),
                    failure: Some(SessionFailure::from_reason(
                        "malformed stream admission reverse dispatch result",
                        "CARRIER_STREAM_PHASE_INVALID",
                        false,
                    )),
                    request_id: None,
                    admission_receipt: None,
                    terminal_receipt: None,
                })),
                true,
            );
        }
        return (DispatchStreamEvent::Admission(Box::new(receipt)), false);
    }
    if result.terminal_receipt.is_some() || result.failure.is_some() {
        return (
            DispatchStreamEvent::Terminal(Box::new(dispatch_result_from_reverse_result(result))),
            true,
        );
    }
    (DispatchStreamEvent::Chunk(result.payload), false)
}

fn dispatch_result_from_reverse_result(
    result: axon_sdk::pb::axon::v1::ReverseDispatchResult,
) -> DispatchResult {
    DispatchResult {
        payload: result.payload,
        result_content_type: result.result_content_type,
        error: result
            .failure
            .as_ref()
            .map(|failure| failure.message.clone()),
        failure: result.failure.as_ref().map(session_failure_from_wire_error),
        request_id: None,
        admission_receipt: result.admission_receipt,
        terminal_receipt: result.terminal_receipt,
    }
}

fn dispatch_result_from_escalation_reply(reply: EscalationReply) -> DispatchResult {
    match reply {
        EscalationReply::Canonical(response) => DispatchResult {
            payload: response.result,
            result_content_type: response.result_content_type,
            error: None,
            failure: None,
            request_id: None,
            admission_receipt: response.admission_receipt,
            terminal_receipt: response.terminal_receipt,
        },
        EscalationReply::Control(RequestOutcome::Ok { result_bytes }) => DispatchResult {
            payload: result_bytes,
            result_content_type: String::new(),
            error: None,
            failure: None,
            request_id: None,
            admission_receipt: None,
            terminal_receipt: None,
        },
        EscalationReply::Control(RequestOutcome::Err { error }) | EscalationReply::Error(error) => {
            let message = session_request_error_message(&error);
            DispatchResult {
                payload: Vec::new(),
                result_content_type: String::new(),
                error: Some(message.clone()),
                failure: Some(SessionFailure::from_reason(
                    &message,
                    session_request_error_code(&error),
                    true,
                )),
                request_id: None,
                admission_receipt: None,
                terminal_receipt: None,
            }
        }
    }
}

fn session_request_error_from_wire_failure(
    failure: axon_sdk::pb::axon::v1::Error,
) -> SessionRequestError {
    match failure.code.as_str() {
        "TARGET_OFFLINE" => SessionRequestError::TargetOffline,
        "PERMISSION_DENIED" => SessionRequestError::PermissionDenied {
            reason: failure.message,
        },
        "UPSTREAM_TIMEOUT" => SessionRequestError::UpstreamTimeout,
        _ => SessionRequestError::UpstreamFailure {
            reason: failure.message,
        },
    }
}

fn session_failure_from_wire_error(error: &axon_sdk::pb::axon::v1::Error) -> SessionFailure {
    SessionFailure::from_reason(
        &error.message,
        if error.code.is_empty() {
            "INVOCATION_FAILED"
        } else {
            error.code.as_str()
        },
        error.retryable,
    )
}

fn session_request_error_code(error: &SessionRequestError) -> &'static str {
    match error {
        SessionRequestError::TargetOffline => "TARGET_OFFLINE",
        SessionRequestError::PermissionDenied { .. } => "PERMISSION_DENIED",
        SessionRequestError::UpstreamFailure { .. } => "UPSTREAM_FAILURE",
        SessionRequestError::UpstreamTimeout => "UPSTREAM_TIMEOUT",
    }
}

fn session_request_error_message(error: &SessionRequestError) -> String {
    match error {
        SessionRequestError::TargetOffline => "target offline".to_string(),
        SessionRequestError::PermissionDenied { reason }
        | SessionRequestError::UpstreamFailure { reason } => reason.clone(),
        SessionRequestError::UpstreamTimeout => "upstream timeout".to_string(),
    }
}

/// Reload-friendly handle on the device's *current*
/// `session.open` bidi up sender. The session supervisor
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
#[derive(Clone, Default)]
pub struct SharedSessionOutbox {
    inner: Arc<Mutex<Option<SessionUpSender>>>,
    ready_hooks: SessionReadyHooks,
}

pub type SessionReadyHook = Arc<dyn Fn() + Send + Sync>;
type SessionReadyHooks = Arc<Mutex<Vec<SessionReadyHook>>>;

impl std::fmt::Debug for SharedSessionOutbox {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SharedSessionOutbox")
            .field("ready", &self.snapshot().is_some())
            .finish_non_exhaustive()
    }
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
        {
            let mut guard = match self.inner.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            *guard = Some(sender);
        }
        let hooks = match self.ready_hooks.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        for hook in hooks {
            hook();
        }
    }

    /// Register work that must be re-driven whenever a new live session
    /// sender is published. The hook runs after the sender is visible and
    /// outside all outbox locks. Registering against an already-ready outbox
    /// invokes the hook immediately.
    pub fn register_ready_hook(&self, hook: SessionReadyHook) {
        {
            let mut hooks = match self.ready_hooks.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            hooks.push(Arc::clone(&hook));
        }
        if self.snapshot().is_some() {
            hook();
        }
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
    /// `UpstreamFailure { reason: "no live session.open bidi" }`.
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
    session_realm: impl Into<String>,
) -> SessionEscalationHandle {
    let (submit_tx, mut submit_rx) = mpsc::channel::<EscalationCommand>(ESCALATION_QUEUE_CAPACITY);
    let handle = SessionEscalationHandle {
        submit: submit_tx,
        correlation: Arc::clone(&correlation),
        session_realm: session_realm.into(),
    };

    tokio::spawn(async move {
        while let Some(command) = submit_rx.recv().await {
            let request = match command {
                EscalationCommand::Request(request) => request,
                EscalationCommand::BidiInput(input) => {
                    let Some(up_tx) = outbox.snapshot() else {
                        continue;
                    };
                    let _ = send_escalation_bidi_input(&up_tx, input).await;
                    continue;
                }
            };
            let EscalationRequest {
                call_id,
                invocation,
                reply,
            } = request;

            let Some(up_tx) = outbox.snapshot() else {
                // No live session — surface a typed upstream-failure
                // outcome so the CLI sees a fast structured error
                // instead of waiting for a hub reconnect.
                let error = SessionRequestError::UpstreamFailure {
                    reason: "no live session.open bidi to escalate canonical_invoke up; \
                                 device-mode daemon's session supervisor has not (yet) \
                                 reconnected"
                        .to_string(),
                };
                fail_reply(reply, error);
                continue;
            };

            register_reply(&correlation, call_id, reply);

            if let Err(err) = send_escalation_request(&up_tx, call_id, invocation).await {
                let mut guard = match correlation.inner.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if let Some(pending) = guard.remove(&call_id) {
                    fail_pending(
                        pending,
                        SessionRequestError::UpstreamFailure {
                            reason: format!(
                                "session up-channel closed mid-push (mid-reconnect?): {err}"
                            ),
                        },
                    );
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
    session_realm: impl Into<String>,
) -> SessionEscalationHandle {
    let (submit_tx, mut submit_rx) = mpsc::channel::<EscalationCommand>(ESCALATION_QUEUE_CAPACITY);
    let handle = SessionEscalationHandle {
        submit: submit_tx,
        correlation: Arc::clone(&correlation),
        session_realm: session_realm.into(),
    };

    tokio::spawn(async move {
        while let Some(command) = submit_rx.recv().await {
            let request = match command {
                EscalationCommand::Request(request) => request,
                EscalationCommand::BidiInput(input) => {
                    let _ = send_escalation_bidi_input(&up_tx, input).await;
                    continue;
                }
            };
            let EscalationRequest {
                call_id,
                invocation,
                reply,
            } = request;
            register_reply(&correlation, call_id, reply);

            if let Err(err) = send_escalation_request(&up_tx, call_id, invocation).await {
                // Up-channel closed — the bidi went away mid-flight.
                // Pull the entry back out and surface upstream
                // failure to the dispatch caller. The consumer
                // task continues to drain so subsequent dispatches
                // surface promptly when a new bidi is available.
                let mut guard = match correlation.inner.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if let Some(pending) = guard.remove(&call_id) {
                    fail_pending(
                        pending,
                        SessionRequestError::UpstreamFailure {
                            reason: format!("session up-channel closed: {err}"),
                        },
                    );
                }
            }
        }
    });

    handle
}

async fn send_escalation_request(
    up_tx: &SessionUpSender,
    call_id: [u8; 16],
    invocation: EscalationInvocation,
) -> Result<(), String> {
    match invocation {
        EscalationInvocation::Canonical(request) => {
            use axon_sdk::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
            use axon_sdk::pb::axon::v1::ReverseDispatchCall;
            up_tx
                .send_payload(UpPayload::ReverseDispatchCall(ReverseDispatchCall {
                    call_id: call_id.to_vec(),
                    request: Some(*request),
                    open_bidi: false,
                }))
                .await
                .map_err(|err| err.to_string())
        }
        EscalationInvocation::CanonicalStream(request) => {
            use axon_sdk::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
            use axon_sdk::pb::axon::v1::{InvokeRequest, ReverseDispatchCall};
            let request = InvokeRequest {
                envelope: request.envelope,
                target: request.target,
                arguments: request.arguments,
                content_type: request.content_type,
                timeout_seconds: request.timeout_seconds,
                metadata: request.metadata,
                payload_ref: request.payload_ref,
                content_envelope: request.content_envelope,
            };
            up_tx
                .send_payload(UpPayload::ReverseDispatchCall(ReverseDispatchCall {
                    call_id: call_id.to_vec(),
                    request: Some(request),
                    open_bidi: false,
                }))
                .await
                .map_err(|err| err.to_string())
        }
        EscalationInvocation::CanonicalBidi(request) => {
            use axon_sdk::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
            use axon_sdk::pb::axon::v1::ReverseDispatchCall;
            up_tx
                .send_payload(UpPayload::ReverseDispatchCall(ReverseDispatchCall {
                    call_id: call_id.to_vec(),
                    request: Some(*request),
                    open_bidi: true,
                }))
                .await
                .map_err(|err| err.to_string())
        }
        EscalationInvocation::DaemonControl { ability_ura, args } => {
            let frame = build_session_request_up_chunk(call_id, &ability_ura, &args);
            up_tx
                .send_binary_chunk(frame)
                .await
                .map_err(|err| err.to_string())
        }
    }
}

async fn send_escalation_bidi_input(
    up_tx: &SessionUpSender,
    input: EscalationBidiInput,
) -> Result<(), String> {
    use axon_sdk::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
    use axon_sdk::pb::axon::v1::reverse_bidi_input::Input;
    use axon_sdk::pb::axon::v1::ReverseBidiInput;

    let call_id = input.call_id;
    let input = match input.input {
        EscalatedBidiInput::Binary(chunk) => Some(Input::BinaryChunk(chunk)),
        EscalatedBidiInput::Control(control) => Some(Input::Control(control)),
    };
    up_tx
        .send_payload(UpPayload::ReverseBidiInput(ReverseBidiInput {
            call_id: call_id.to_vec(),
            input,
        }))
        .await
        .map_err(|err| err.to_string())
}

fn register_reply(
    correlation: &Arc<EscalationCorrelation>,
    call_id: [u8; 16],
    reply: EscalationReplySink,
) {
    match reply {
        EscalationReplySink::Unary(sender) => {
            correlation.register_unary(call_id, sender);
        }
        EscalationReplySink::Stream { events, accepted } => {
            correlation.register_stream(call_id, events);
            let _ = accepted.send(Ok(()));
        }
    }
}

fn fail_reply(reply: EscalationReplySink, error: SessionRequestError) {
    match reply {
        EscalationReplySink::Unary(sender) => {
            let _ = sender.send(EscalationReply::Error(error));
        }
        EscalationReplySink::Stream { accepted, .. } => {
            let _ = accepted.send(Err(error));
        }
    }
}

fn fail_pending(pending: PendingEscalation, error: SessionRequestError) {
    match pending {
        PendingEscalation::Unary(sender) => {
            let _ = sender.send(EscalationReply::Error(error));
        }
        PendingEscalation::Stream(sender) => {
            let message = session_request_error_message(&error);
            let _ = sender.try_send(DispatchStreamEvent::Terminal(Box::new(DispatchResult {
                payload: Vec::new(),
                result_content_type: String::new(),
                error: Some(message.clone()),
                failure: Some(SessionFailure::from_reason(
                    &message,
                    session_request_error_code(&error),
                    true,
                )),
                request_id: None,
                admission_receipt: None,
                terminal_receipt: None,
            })));
        }
    }
}

/// Build the `InvokeBidiUp` frame that wraps a
/// `SessionDispatch::Request` JSON in a `BinaryChunk` payload.
/// Mirrors what `dial_and_run_session` writes for non-Request
/// frames + matches the wire shape PR-N6 §"Wire shape" locks.
fn build_session_request_up_chunk(
    call_id: [u8; 16],
    ability_ura: &str,
    args: &[u8],
) -> axon_sdk::pb::axon::v1::BinaryChunk {
    use crate::daemon::invocation::bidi::session_wire::{SessionContentEnvelope, SessionDispatch};
    use axon_sdk::pb::axon::v1::BinaryChunk;

    let dispatch = SessionDispatch::Request {
        call_id,
        ability_ura: ability_ura.to_string(),
        args: args.to_vec(),
        args_content_envelope: SessionContentEnvelope::plaintext_json(),
    };
    // serde encoding of an owned-fields enum cannot fail here;
    // the unwrap is justified by the typed enum domain.
    let data = dispatch
        .encode_frame()
        .expect("encode SessionDispatch::Request");

    BinaryChunk {
        stream_id: SESSION_STREAM_ID,
        data,
        ..BinaryChunk::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axon_sdk::pb::axon::v1::{InvocationReceipt, InvocationState, InvokeBidiUp};

    #[test]
    fn reverse_unary_reply_preserves_terminal_state_and_json_content_type() {
        let reply = reverse_unary_reply(axon_sdk::pb::axon::v1::ReverseDispatchResult {
            payload: br#"{"abilities":[]}"#.to_vec(),
            result_content_type: "application/json".to_string(),
            terminal: true,
            admission_receipt: Some(InvocationReceipt {
                state: InvocationState::Running as i32,
                ..InvocationReceipt::default()
            }),
            terminal_receipt: Some(InvocationReceipt {
                state: InvocationState::Completed as i32,
                ..InvocationReceipt::default()
            }),
            ..axon_sdk::pb::axon::v1::ReverseDispatchResult::default()
        });

        match reply {
            EscalationReply::Canonical(response) => {
                assert_eq!(response.state, InvocationState::Completed as i32);
                assert_eq!(response.result_content_type, "application/json");
                assert_eq!(response.result, br#"{"abilities":[]}"#);
            }
            other => panic!("expected canonical reverse unary reply, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn escalate_resolves_when_correlation_completes() {
        // Wire-up: spawn consumer + a fake "hub" task that drains
        // up_rx, decodes the Request frame, and feeds the
        // matching RequestResult back into the correlation table.
        let correlation = EscalationCorrelation::new();
        let (up_tx, mut up_rx) = mpsc::channel::<axon_sdk::pb::axon::v1::InvokeBidiUp>(8);
        let handle = spawn_escalation_consumer(
            Arc::clone(&correlation),
            SessionUpSender::new(up_tx),
            "test-realm",
        );

        let correlation_for_hub = Arc::clone(&correlation);
        tokio::spawn(async move {
            while let Some(frame) = up_rx.recv().await {
                use axon_sdk::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
                let chunk = match frame.payload {
                    Some(UpPayload::BinaryChunk(c)) => c,
                    _ => continue,
                };
                let dispatch: crate::daemon::invocation::bidi::session_wire::SessionDispatch =
                    serde_json::from_slice(&chunk.data).expect("decode");
                if let crate::daemon::invocation::bidi::session_wire::SessionDispatch::Request {
                    call_id,
                    ..
                } = dispatch
                {
                    correlation_for_hub.complete(
                        call_id,
                        EscalationReply::Control(RequestOutcome::Ok {
                            result_bytes: b"hub-resolved".to_vec(),
                        }),
                    );
                }
            }
        });

        let outcome = handle
            .escalate("federation.resolve_key".into(), b"{}".to_vec())
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
        let (up_tx, _up_rx_held) = mpsc::channel::<axon_sdk::pb::axon::v1::InvokeBidiUp>(8);
        let handle = spawn_escalation_consumer(
            Arc::clone(&correlation),
            SessionUpSender::new(up_tx),
            "test-realm",
        );

        let outcome = handle
            .escalate_with_timeout(
                "federation.resolve_key".into(),
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
        let (up_tx, up_rx) = mpsc::channel::<axon_sdk::pb::axon::v1::InvokeBidiUp>(8);
        // Drop the receiver immediately so the consumer's send
        // fails on the very first item.
        drop(up_rx);
        let handle = spawn_escalation_consumer(
            Arc::clone(&correlation),
            SessionUpSender::new(up_tx),
            "test-realm",
        );

        let outcome = handle
            .escalate_with_timeout(
                "federation.resolve_key".into(),
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
            EscalationReply::Control(RequestOutcome::Ok {
                result_bytes: vec![],
            }),
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
        let handle =
            spawn_escalation_consumer_with_outbox(Arc::clone(&correlation), outbox, "test-realm");

        let outcome = handle
            .escalate_with_timeout(
                "federation.resolve_key".into(),
                b"{}".to_vec(),
                Duration::from_secs(2),
            )
            .await;
        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamFailure { reason },
            } => {
                assert!(
                    reason.contains("no live session.open bidi"),
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
        let handle = spawn_escalation_consumer_with_outbox(
            Arc::clone(&correlation),
            outbox.clone(),
            "test-realm",
        );

        let (up_tx, mut up_rx) = mpsc::channel::<InvokeBidiUp>(8);
        outbox.set(SessionUpSender::new(up_tx));

        let correlation_for_hub = Arc::clone(&correlation);
        tokio::spawn(async move {
            while let Some(frame) = up_rx.recv().await {
                use axon_sdk::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
                let chunk = match frame.payload {
                    Some(UpPayload::BinaryChunk(c)) => c,
                    _ => continue,
                };
                let dispatch: crate::daemon::invocation::bidi::session_wire::SessionDispatch =
                    serde_json::from_slice(&chunk.data).expect("decode");
                if let crate::daemon::invocation::bidi::session_wire::SessionDispatch::Request {
                    call_id,
                    ..
                } = dispatch
                {
                    correlation_for_hub.complete(
                        call_id,
                        EscalationReply::Control(RequestOutcome::Ok {
                            result_bytes: b"hub-via-outbox".to_vec(),
                        }),
                    );
                }
            }
        });

        let outcome = handle
            .escalate("federation.resolve_key".into(), b"{}".to_vec())
            .await;
        match outcome {
            RequestOutcome::Ok { result_bytes } => {
                assert_eq!(result_bytes, b"hub-via-outbox");
            }
            other => panic!("expected Ok via outbox, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bidi_escalation_sends_open_then_input_on_same_reverse_call_id() {
        let correlation = EscalationCorrelation::new();
        let (up_tx, mut up_rx) = mpsc::channel::<InvokeBidiUp>(8);
        let handle = spawn_escalation_consumer(
            Arc::clone(&correlation),
            SessionUpSender::new(up_tx),
            "test-realm",
        );

        let bidi = handle
            .escalate_bidi(axon_sdk::pb::axon::v1::InvokeRequest::default())
            .await
            .expect("bidi escalation opens");
        let input = bidi.input_sender();

        let open = tokio::time::timeout(Duration::from_secs(2), up_rx.recv())
            .await
            .expect("open frame arrives")
            .expect("open frame present");
        let call_id = match open.payload {
            Some(axon_sdk::pb::axon::v1::invoke_bidi_up::Payload::ReverseDispatchCall(call)) => {
                assert!(call.open_bidi, "reverse bidi open must set open_bidi");
                assert_eq!(call.call_id.len(), 16);
                call.call_id
            }
            other => panic!("expected ReverseDispatchCall open, got {other:?}"),
        };

        input
            .send_binary(axon_sdk::pb::axon::v1::BinaryChunk {
                stream_id: 1,
                data: b"frame".to_vec(),
                ..Default::default()
            })
            .await
            .expect("input queues");

        let input_frame = tokio::time::timeout(Duration::from_secs(2), up_rx.recv())
            .await
            .expect("input frame arrives")
            .expect("input frame present");
        match input_frame.payload {
            Some(axon_sdk::pb::axon::v1::invoke_bidi_up::Payload::ReverseBidiInput(input)) => {
                assert_eq!(input.call_id, call_id);
                match input.input {
                    Some(axon_sdk::pb::axon::v1::reverse_bidi_input::Input::BinaryChunk(chunk)) => {
                        assert_eq!(chunk.stream_id, 1);
                        assert_eq!(chunk.data, b"frame");
                    }
                    other => panic!("expected reverse bidi binary input, got {other:?}"),
                }
            }
            other => panic!("expected ReverseBidiInput, got {other:?}"),
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
        let handle = spawn_escalation_consumer_with_outbox(
            Arc::clone(&correlation),
            outbox.clone(),
            "test-realm",
        );

        let (up_tx, _up_rx_held) = mpsc::channel::<InvokeBidiUp>(8);
        outbox.set(SessionUpSender::new(up_tx));
        outbox.clear();

        let outcome = handle
            .escalate_with_timeout(
                "federation.resolve_key".into(),
                b"{}".to_vec(),
                Duration::from_secs(2),
            )
            .await;
        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamFailure { reason },
            } => {
                assert!(
                    reason.contains("no live session.open bidi"),
                    "reason should cite missing session; got {reason}",
                );
            }
            other => panic!("expected UpstreamFailure after clear, got {other:?}"),
        }
    }
}
