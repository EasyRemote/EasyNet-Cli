//! The CLI-side async driver for the sans-IO invocation lifecycle (AXON-RFC-008 S3).
//!
//! [`crate::services::invocation_transport`] historically ran one bespoke
//! `async fn` per transport geometry, each re-deriving the same lifecycle. This
//! driver replaces that: it owns the single sans-IO
//! [`InvocationLifecycle`](easynet_axon::invocation::InvocationLifecycle) core
//! plus a [`Transport`] and an [`LifecycleExecutor`], and runs the universal
//! pump:
//!
//! ```text
//! recv frame  ->  decode to Event  ->  core.on_event()  ->  execute Actions
//! ```
//!
//! The core decides *what* happens (protocol semantics); the executor supplies
//! the *live daemon facts* (run admission, resolve a route, dispatch to a
//! target, project a receipt) and the transport carries bytes. Each geometry
//! (unary / stream / bidi / local-session) becomes a thin `Transport` +
//! `LifecycleExecutor` pair wired into this one loop — no per-geometry lifecycle.

// The driver is the S4 wiring target: until each geometry's service arm is
// switched over (S4-unary/stream/bidi/session), its only callers are the unit
// tests below. The allow is scoped to this module and removed as S4 lands.
#![allow(dead_code)]

use async_trait::async_trait;
use easynet_axon::invocation::frame::{ControlSignal, TransportError, TransportFrame};
use easynet_axon::invocation::lifecycle::{
    Action, Event, FailureReason, InvocationLifecycle, ReceiptKind, State, TerminalKind,
};
use easynet_axon::invocation::route::{ResolvedRoute, RouteError};
use easynet_axon::invocation::transport::Transport;

/// Executes the side-effecting lifecycle [`Action`]s against live daemon facts.
///
/// The sans-IO core never touches presence, sessions, the resolver, or the
/// ledger; it emits intents and this executor — implemented per geometry in the
/// CLI — fulfils them and feeds the outcomes back as [`Event`]s. Keeping it a
/// trait makes the driver loop testable with a mock that records calls.
#[async_trait]
pub(crate) trait LifecycleExecutor: Send {
    /// Run the admission gate; returns the resulting admission event.
    async fn run_admission(&mut self, envelope: &[u8], args: &[u8]) -> Event;

    /// Resolve the route via the daemon's live lookup; returns Resolved/Rejected.
    async fn resolve_route(&mut self) -> Result<ResolvedRoute, RouteError>;

    /// Dispatch to the resolved target; returns the acceptance event.
    async fn dispatch_to(&mut self, route: &ResolvedRoute) -> Event;

    /// Project the terminal receipt per invariant I4 (the executor decides the
    /// legitimate source; it never lets the core mint a canonical receipt).
    async fn project_receipt(&mut self, kind: ReceiptKind);

    /// Best-effort ledger persistence of a hub-forwarding record.
    async fn persist_ledger_best_effort(&mut self);
}

/// Drives one invocation to settlement over a [`Transport`] using the sans-IO core.
pub(crate) struct LifecycleDriver<T: Transport, E: LifecycleExecutor> {
    core: InvocationLifecycle,
    transport: T,
    executor: E,
}

impl<T: Transport, E: LifecycleExecutor> LifecycleDriver<T, E> {
    /// Build a driver over a transport and executor, starting from `Idle`.
    pub(crate) fn new(transport: T, executor: E) -> Self {
        Self { core: InvocationLifecycle::new(), transport, executor }
    }

    /// The current lifecycle state (for observation / tests).
    pub(crate) fn state(&self) -> State {
        self.core.state()
    }

    /// Run the invocation to a terminal state.
    ///
    /// Pumps the first inbound frame in as `InvocationReceived`, then loops:
    /// execute the core's actions (which may produce follow-up events fed back
    /// synchronously), and when the core is between actions, pull the next
    /// transport frame. Returns once the core settles.
    pub(crate) async fn run(&mut self) -> Result<State, TransportError> {
        // Open the invocation with the first frame.
        let first = self.transport.recv_frame().await?;
        let Some(ev) = decode_inbound(first) else {
            // A non-admission opening frame is a transport-level protocol
            // violation, not a lifecycle event: the core only accepts an
            // InvocationReceived from Idle. Fail closed at the driver without
            // ever entering the lifecycle, surfacing a typed error to the peer.
            let _ = self
                .transport
                .send_frame(TransportFrame::Error {
                    code: "protocol_violation".into(),
                    message: "opening frame was not an admission frame".into(),
                    retryable: false,
                })
                .await;
            return Ok(State::Settled(TerminalKind::Failed(
                FailureReason::AdmissionDenied,
            )));
        };
        self.pump(ev).await
    }

    /// Feed one event into the core and execute the resulting actions until the
    /// core needs another inbound frame or has settled.
    async fn pump(&mut self, mut event: Event) -> Result<State, TransportError> {
        loop {
            let actions = self.core.on_event(event);
            // Execute every action; some yield a follow-up event to feed back.
            let mut next_event: Option<Event> = None;
            for action in actions {
                if let Some(ev) = self.execute(action).await? {
                    // Last follow-up event wins; actions that produce events
                    // (admission, route, dispatch) are mutually exclusive per
                    // transition, so at most one is set per action batch.
                    next_event = Some(ev);
                }
            }

            if self.core.is_settled() {
                return Ok(self.core.state());
            }

            match next_event {
                // A follow-up event (admission/route/dispatch outcome) drives
                // the next transition without touching the transport.
                Some(ev) => event = ev,
                // No follow-up: the core is waiting on the peer. Pull a frame.
                None => {
                    let frame = self.transport.recv_frame().await?;
                    event = decode_dispatch_frame(frame);
                }
            }
        }
    }

    /// Execute a single action, returning an optional follow-up event.
    async fn execute(&mut self, action: Action) -> Result<Option<Event>, TransportError> {
        match action {
            Action::RunAdmission { envelope, args } => {
                Ok(Some(self.executor.run_admission(&envelope, &args).await))
            }
            Action::ResolveRoute => {
                let ev = match self.executor.resolve_route().await {
                    Ok(route) => Event::RouteResolved(route),
                    Err(err) => Event::RouteRejected(err),
                };
                Ok(Some(ev))
            }
            Action::DispatchTo { target: _ } => {
                // The executor holds the resolved route (captured during
                // resolve_route); it dispatches and reports acceptance.
                let route = self
                    .core
                    .context()
                    .route
                    .clone()
                    .expect("DispatchTo implies a resolved route in context");
                Ok(Some(self.executor.dispatch_to(&route).await))
            }
            Action::SendFrame { payload } => {
                self.transport
                    .send_frame(TransportFrame::Chunk { payload })
                    .await?;
                Ok(None)
            }
            Action::ProjectReceipt { kind } => {
                self.executor.project_receipt(kind).await;
                Ok(None)
            }
            Action::PersistLedgerBestEffort => {
                self.executor.persist_ledger_best_effort().await;
                Ok(None)
            }
            Action::EmitError { reason } => {
                // Surface the typed failure to the caller; ignore a send error
                // on an already-terminal invocation (transport diagnostic only).
                let _ = self
                    .transport
                    .send_frame(TransportFrame::Error {
                        code: format!("{reason:?}"),
                        message: format!("{reason:?}"),
                        retryable: false,
                    })
                    .await;
                Ok(None)
            }
            Action::CloseTransport => Ok(None),
        }
    }
}

/// Decode the opening inbound frame into an `InvocationReceived` event.
fn decode_inbound(frame: TransportFrame) -> Option<Event> {
    match frame {
        TransportFrame::Admission { envelope, args } => {
            Some(Event::InvocationReceived { envelope, args })
        }
        _ => None,
    }
}

/// Decode a mid-flight transport frame into the corresponding dispatch event.
fn decode_dispatch_frame(frame: TransportFrame) -> Event {
    match frame {
        TransportFrame::Chunk { payload } => Event::DispatchChunk { payload },
        TransportFrame::Result { payload, receipt } => {
            Event::DispatchTerminal { payload, receipt }
        }
        TransportFrame::Error { .. } => Event::DispatchFailed,
        TransportFrame::Control(ControlSignal::Cancel) => Event::CallerCancelled,
        TransportFrame::Control(ControlSignal::Eof) => Event::TargetPeerClosed,
        // An unexpected re-admission frame mid-flight is a peer protocol error.
        TransportFrame::Admission { .. } => Event::TargetPeerClosed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use easynet_axon::invocation::lifecycle::TerminalKind;
    use easynet_axon::invocation::route::{DispatchTarget, RouteGeometry, RouteProfile};
    use std::collections::VecDeque;

    /// A scripted transport: yields queued inbound frames, records sent frames.
    struct MockTransport {
        inbound: VecDeque<TransportFrame>,
        sent: Vec<TransportFrame>,
    }

    impl MockTransport {
        fn new(inbound: Vec<TransportFrame>) -> Self {
            Self { inbound: inbound.into(), sent: Vec::new() }
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn recv_frame(&mut self) -> Result<TransportFrame, TransportError> {
            self.inbound.pop_front().ok_or(TransportError {
                code: "eof".into(),
                message: "no more frames".into(),
                retryable: false,
            })
        }
        async fn send_frame(&mut self, frame: TransportFrame) -> Result<(), TransportError> {
            self.sent.push(frame);
            Ok(())
        }
    }

    /// A scripted executor recording calls and returning configured outcomes.
    #[derive(Default)]
    struct MockExecutor {
        admission_granted: bool,
        route: Option<ResolvedRoute>,
        projected: Vec<ReceiptKind>,
        admission_calls: usize,
        dispatch_calls: usize,
    }

    fn sample_route() -> ResolvedRoute {
        ResolvedRoute {
            query_name: "agent.demo".into(),
            owner_ura: "ura:agent:demo".into(),
            callee_ura: "ura:agent:demo".into(),
            execution_host_ura: "ura:hub:local".into(),
            ability_ura: "ura:ability:demo".into(),
            route_ura: "ura:route:demo".into(),
            dispatch_name: "demo".into(),
            profile: RouteProfile::Production,
            target: DispatchTarget {
                geometry: RouteGeometry::LocalHub,
                opaque: "slot-7".into(),
            },
        }
    }

    #[async_trait]
    impl LifecycleExecutor for MockExecutor {
        async fn run_admission(&mut self, _envelope: &[u8], _args: &[u8]) -> Event {
            self.admission_calls += 1;
            if self.admission_granted {
                Event::AdmissionGranted
            } else {
                Event::AdmissionDenied
            }
        }
        async fn resolve_route(&mut self) -> Result<ResolvedRoute, RouteError> {
            self.route
                .clone()
                .ok_or(RouteError::NotFound { query_name: "agent.demo".into() })
        }
        async fn dispatch_to(&mut self, _route: &ResolvedRoute) -> Event {
            self.dispatch_calls += 1;
            Event::DispatchAccepted
        }
        async fn project_receipt(&mut self, kind: ReceiptKind) {
            self.projected.push(kind);
        }
        async fn persist_ledger_best_effort(&mut self) {}
    }

    #[tokio::test]
    async fn happy_path_unary_settles_completed() {
        let transport = MockTransport::new(vec![
            TransportFrame::Admission { envelope: vec![1], args: vec![2] },
            // After dispatch acceptance, the core pulls the next frame: terminal.
            TransportFrame::Result { payload: vec![9], receipt: vec![7] },
        ]);
        let executor = MockExecutor {
            admission_granted: true,
            route: Some(sample_route()),
            ..Default::default()
        };
        let mut driver = LifecycleDriver::new(transport, executor);
        let state = driver.run().await.expect("driver runs");
        assert_eq!(state, State::Settled(TerminalKind::Completed));
        // Completed receipt was projected exactly once.
        assert_eq!(driver.executor.projected, vec![ReceiptKind::Completed]);
        assert_eq!(driver.executor.admission_calls, 1);
        assert_eq!(driver.executor.dispatch_calls, 1);
    }

    #[tokio::test]
    async fn admission_denied_settles_without_dispatch() {
        let transport = MockTransport::new(vec![TransportFrame::Admission {
            envelope: vec![1],
            args: vec![2],
        }]);
        let executor = MockExecutor { admission_granted: false, ..Default::default() };
        let mut driver = LifecycleDriver::new(transport, executor);
        let state = driver.run().await.expect("driver runs");
        assert!(matches!(state, State::Settled(TerminalKind::Failed(_))));
        // Never dispatched; a failure receipt was projected.
        assert_eq!(driver.executor.dispatch_calls, 0);
        assert_eq!(driver.executor.projected.len(), 1);
    }

    #[tokio::test]
    async fn route_rejected_settles_failed() {
        let transport = MockTransport::new(vec![TransportFrame::Admission {
            envelope: vec![1],
            args: vec![2],
        }]);
        let executor = MockExecutor {
            admission_granted: true,
            route: None, // resolve_route -> NotFound
            ..Default::default()
        };
        let mut driver = LifecycleDriver::new(transport, executor);
        let state = driver.run().await.expect("driver runs");
        assert!(matches!(state, State::Settled(TerminalKind::Failed(_))));
        assert_eq!(driver.executor.dispatch_calls, 0);
    }

    #[tokio::test]
    async fn non_admission_opening_frame_fails_closed() {
        let transport = MockTransport::new(vec![TransportFrame::Chunk { payload: vec![1] }]);
        let executor = MockExecutor { admission_granted: true, ..Default::default() };
        let mut driver = LifecycleDriver::new(transport, executor);
        let state = driver.run().await.expect("driver runs");
        // Opening with a non-admission frame must not execute admission.
        assert_eq!(driver.executor.admission_calls, 0);
        assert!(matches!(state, State::Settled(TerminalKind::Failed(_))));
    }
}
