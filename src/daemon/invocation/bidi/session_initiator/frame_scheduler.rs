// EasyNet CLI — session.open inbound fair scheduler
// =================================================
//
// File: src/daemon/invocation/bidi/session_initiator/frame_scheduler.rs
// Description: Session-owned bounded demultiplexer for Hub → Device frames.
//
// Protocol responsibility:
// - Preserve FIFO order inside one remote bidi call.
// - Keep control replies and independent calls off a stalled bidi data lane.
// - Bound both per-call and whole-session buffered work.
//
// Implementation approach: one session actor, one in-flight future per call,
// FIFO pending queues, and global/per-call admission permits.
//
// Usage contract: `run_live_session` constructs exactly one scheduler for one
// carrier scope and routes every validated-sequence down frame through it.
//
// Architectural position: EasyNet daemon transport scheduling. Axon remains
// the owner of Invocation admission, execution, and terminal receipts.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use axon_sdk::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
use axon_sdk::pb::axon::v1::InvokeBidiDown;
use futures::stream::FuturesUnordered;
use futures::StreamExt as _;
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

use super::{SessionDispatchError, SessionFrameDispatcher, SessionUpSender};
use crate::daemon::invocation::bidi::session_wire::SessionDispatch;
use crate::daemon::invocation::bidi::state::presence::DISPATCH_CHANNEL_CAPACITY;

/// One stalled call may retain a useful burst, but cannot consume the whole
/// carrier budget. This is twice the LocalRuntime opening-ingress capacity, so
/// the 33rd frame has a bounded scheduler slot while admission is unresolved.
const BIDI_FRAMES_PER_CALL: usize = 64;

#[derive(Debug, thiserror::Error)]
pub(super) enum SessionFrameScheduleError {
    #[error(transparent)]
    Dispatch(#[from] SessionDispatchError),
    #[error(
        "remote bidi call_id={call_id} exceeded its bounded scheduler budget of {limit} frames"
    )]
    CallSaturated { call_id: u64, limit: usize },
    #[error(
        "session bidi scheduler exceeded its bounded budget of {limit} frames while admitting call_id={call_id}"
    )]
    SessionSaturated { call_id: u64, limit: usize },
    #[error("session bidi scheduler closed while admitting call_id={call_id}")]
    Closed { call_id: u64 },
}

/// Routes control frames inline and bidi input through one session-owned
/// bounded actor. The actor owns at most one in-flight dispatcher future per
/// call, which is both the per-call FIFO barrier and the fairness boundary.
pub(super) struct SessionDownFrameScheduler<D: SessionFrameDispatcher> {
    dispatcher: Arc<D>,
    outbound: SessionUpSender,
    bidi_tx: mpsc::Sender<ScheduledBidiFrame>,
    budget: Arc<SessionBidiBudget>,
    task: tokio::task::JoinHandle<()>,
}

impl<D: SessionFrameDispatcher> SessionDownFrameScheduler<D> {
    pub(super) fn spawn(dispatcher: Arc<D>, outbound: SessionUpSender) -> Self {
        let budget = Arc::new(SessionBidiBudget::new());
        let (bidi_tx, bidi_rx) = mpsc::channel(DISPATCH_CHANNEL_CAPACITY);
        let task = tokio::spawn(run_bidi_scheduler(
            Arc::clone(&dispatcher),
            outbound.clone(),
            bidi_rx,
        ));
        Self {
            dispatcher,
            outbound,
            bidi_tx,
            budget,
            task,
        }
    }

    pub(super) async fn route(
        &self,
        frame: InvokeBidiDown,
    ) -> Result<(), SessionFrameScheduleError> {
        let Some(call_id) = bidi_input_call_id(&frame) else {
            return self
                .dispatcher
                .handle_down(frame, &self.outbound)
                .await
                .map_err(SessionFrameScheduleError::Dispatch);
        };

        let permit = self.budget.reserve(call_id)?;
        let scheduled = ScheduledBidiFrame {
            call_id,
            frame,
            _permit: permit,
        };
        match self.bidi_tx.try_send(scheduled) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_scheduled)) => {
                Err(SessionFrameScheduleError::SessionSaturated {
                    call_id,
                    limit: DISPATCH_CHANNEL_CAPACITY,
                })
            }
            Err(mpsc::error::TrySendError::Closed(_scheduled)) => {
                Err(SessionFrameScheduleError::Closed { call_id })
            }
        }
    }
}

impl<D: SessionFrameDispatcher> Drop for SessionDownFrameScheduler<D> {
    fn drop(&mut self) {
        // The scheduler is scoped to one exact carrier generation. Abort it
        // before DispatcherSessionGuard retires that carrier's call registry.
        self.task.abort();
    }
}

fn bidi_input_call_id(frame: &InvokeBidiDown) -> Option<u64> {
    let Some(DownPayload::BinaryChunk(chunk)) = frame.payload.as_ref() else {
        return None;
    };
    match SessionDispatch::decode_frame(&chunk.data).ok()? {
        SessionDispatch::BidiInput { call_id, .. } => Some(call_id),
        _ => None,
    }
}

struct SessionBidiBudget {
    global: Arc<Semaphore>,
    per_call: Mutex<HashMap<u64, usize>>,
}

impl SessionBidiBudget {
    fn new() -> Self {
        Self {
            global: Arc::new(Semaphore::new(DISPATCH_CHANNEL_CAPACITY)),
            per_call: Mutex::new(HashMap::new()),
        }
    }

    fn reserve(
        self: &Arc<Self>,
        call_id: u64,
    ) -> Result<SessionBidiPermit, SessionFrameScheduleError> {
        let global = Arc::clone(&self.global).try_acquire_owned().map_err(|_| {
            SessionFrameScheduleError::SessionSaturated {
                call_id,
                limit: DISPATCH_CHANNEL_CAPACITY,
            }
        })?;
        let mut per_call = self
            .per_call
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let retained = per_call.entry(call_id).or_default();
        if *retained >= BIDI_FRAMES_PER_CALL {
            return Err(SessionFrameScheduleError::CallSaturated {
                call_id,
                limit: BIDI_FRAMES_PER_CALL,
            });
        }
        *retained += 1;
        drop(per_call);
        Ok(SessionBidiPermit {
            budget: Arc::clone(self),
            call_id,
            _global: global,
        })
    }

    fn release_call(&self, call_id: u64) {
        let mut per_call = self
            .per_call
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(retained) = per_call.get_mut(&call_id) else {
            return;
        };
        *retained -= 1;
        if *retained == 0 {
            per_call.remove(&call_id);
        }
    }
}

struct SessionBidiPermit {
    budget: Arc<SessionBidiBudget>,
    call_id: u64,
    _global: OwnedSemaphorePermit,
}

impl Drop for SessionBidiPermit {
    fn drop(&mut self) {
        self.budget.release_call(self.call_id);
    }
}

struct ScheduledBidiFrame {
    call_id: u64,
    frame: InvokeBidiDown,
    _permit: SessionBidiPermit,
}

struct CompletedBidiFrame {
    call_id: u64,
    result: Result<(), SessionDispatchError>,
}

async fn dispatch_bidi_frame<D: SessionFrameDispatcher>(
    dispatcher: Arc<D>,
    outbound: SessionUpSender,
    scheduled: ScheduledBidiFrame,
) -> CompletedBidiFrame {
    let ScheduledBidiFrame {
        call_id,
        frame,
        _permit,
    } = scheduled;
    let result = dispatcher.handle_down(frame, &outbound).await;
    drop(_permit);
    CompletedBidiFrame { call_id, result }
}

async fn run_bidi_scheduler<D: SessionFrameDispatcher>(
    dispatcher: Arc<D>,
    outbound: SessionUpSender,
    mut bidi_rx: mpsc::Receiver<ScheduledBidiFrame>,
) {
    let mut pending = HashMap::<u64, VecDeque<ScheduledBidiFrame>>::new();
    let mut active = HashSet::<u64>::new();
    let mut in_flight = FuturesUnordered::new();
    let mut input_closed = false;

    loop {
        if input_closed && in_flight.is_empty() {
            break;
        }
        tokio::select! {
            maybe_scheduled = bidi_rx.recv(), if !input_closed => {
                match maybe_scheduled {
                    Some(scheduled) if active.insert(scheduled.call_id) => {
                        in_flight.push(dispatch_bidi_frame(
                            Arc::clone(&dispatcher),
                            outbound.clone(),
                            scheduled,
                        ));
                    }
                    Some(scheduled) => {
                        pending.entry(scheduled.call_id).or_default().push_back(scheduled);
                    }
                    None => input_closed = true,
                }
            }
            Some(completed) = in_flight.next(), if !in_flight.is_empty() => {
                if let Err(error) = completed.result {
                    crate::op_event!(
                        component = session,
                        kind = frame_dispatch_error,
                        call_id = completed.call_id,
                        error = error.to_string(),
                        message = "continuing",
                    );
                }
                let next = pending
                    .get_mut(&completed.call_id)
                    .and_then(VecDeque::pop_front);
                if pending
                    .get(&completed.call_id)
                    .is_some_and(VecDeque::is_empty)
                {
                    pending.remove(&completed.call_id);
                }
                if let Some(next) = next {
                    in_flight.push(dispatch_bidi_frame(
                        Arc::clone(&dispatcher),
                        outbound.clone(),
                        next,
                    ));
                } else {
                    active.remove(&completed.call_id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use axon_sdk::pb::axon::v1::{
        invoke_bidi_down::Payload, BidiControl, BinaryChunk, DispatchCall, InvocationCallMode,
    };

    use super::*;
    use crate::daemon::invocation::bidi::session_wire::RequestOutcome;

    struct OpeningTrustSyncDispatcher {
        input_gate: tokio::sync::watch::Sender<bool>,
        input_started: AtomicUsize,
        inputs: Mutex<Vec<u8>>,
        request_results: AtomicUsize,
        opened_calls: Mutex<Vec<u64>>,
        heartbeats: AtomicUsize,
    }

    impl OpeningTrustSyncDispatcher {
        fn new() -> Self {
            let (input_gate, _rx) = tokio::sync::watch::channel(false);
            Self {
                input_gate,
                input_started: AtomicUsize::new(0),
                inputs: Mutex::new(Vec::new()),
                request_results: AtomicUsize::new(0),
                opened_calls: Mutex::new(Vec::new()),
                heartbeats: AtomicUsize::new(0),
            }
        }

        async fn wait_for_input_start(&self) {
            tokio::time::timeout(Duration::from_secs(1), async {
                while self.input_started.load(Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("one data-lane future starts while Opening is unresolved");
        }

        async fn wait_for_all_inputs(&self, expected: usize) {
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if self
                        .inputs
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .len()
                        == expected
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("all scheduled bidi inputs drain after trust sync");
        }
    }

    #[async_trait::async_trait]
    impl SessionFrameDispatcher for OpeningTrustSyncDispatcher {
        async fn handle_down(
            &self,
            frame: InvokeBidiDown,
            _outbound: &SessionUpSender,
        ) -> Result<(), SessionDispatchError> {
            match frame.payload {
                Some(Payload::DispatchCall(call)) => {
                    self.opened_calls
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(call.call_id);
                }
                Some(Payload::Control(_)) => {
                    self.heartbeats.fetch_add(1, Ordering::SeqCst);
                }
                Some(Payload::BinaryChunk(chunk)) => {
                    match SessionDispatch::decode_frame(&chunk.data).map_err(|error| {
                        SessionDispatchError::Other(format!("decode test frame: {error}"))
                    })? {
                        SessionDispatch::BidiInput { payload, .. } => {
                            self.input_started.fetch_add(1, Ordering::SeqCst);
                            let mut gate = self.input_gate.subscribe();
                            if !*gate.borrow() {
                                gate.wait_for(|ready| *ready).await.map_err(|_| {
                                    SessionDispatchError::Other(
                                        "test Opening gate closed".to_string(),
                                    )
                                })?;
                            }
                            self.inputs
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .push(payload[0]);
                        }
                        SessionDispatch::RequestResult { .. } => {
                            self.request_results.fetch_add(1, Ordering::SeqCst);
                        }
                        other => {
                            return Err(SessionDispatchError::Other(format!(
                                "unexpected test dispatch: {other:?}"
                            )));
                        }
                    }
                }
                other => {
                    return Err(SessionDispatchError::Other(format!(
                        "unexpected test payload: {other:?}"
                    )));
                }
            }
            Ok(())
        }
    }

    fn dispatch_call(call_id: u64) -> InvokeBidiDown {
        InvokeBidiDown {
            payload: Some(Payload::DispatchCall(DispatchCall {
                call_id,
                call_mode: InvocationCallMode::Bidi.into(),
                ..DispatchCall::default()
            })),
            ..InvokeBidiDown::default()
        }
    }

    fn session_dispatch_frame(dispatch: SessionDispatch) -> InvokeBidiDown {
        InvokeBidiDown {
            payload: Some(Payload::BinaryChunk(BinaryChunk {
                data: dispatch.encode_frame().expect("test dispatch encodes"),
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        }
    }

    fn bidi_input(call_id: u64, sequence: u8) -> InvokeBidiDown {
        session_dispatch_frame(SessionDispatch::BidiInput {
            call_id,
            payload: vec![sequence],
            eof: false,
        })
    }

    fn request_result() -> InvokeBidiDown {
        session_dispatch_frame(SessionDispatch::RequestResult {
            call_id: [7; 16],
            outcome: RequestOutcome::Ok {
                result_bytes: b"hub-attested-key".to_vec(),
            },
        })
    }

    fn heartbeat() -> InvokeBidiDown {
        InvokeBidiDown {
            payload: Some(Payload::Control(BidiControl::default())),
            ..InvokeBidiDown::default()
        }
    }

    #[tokio::test]
    async fn opening_bidi_burst_cannot_block_trust_reply_other_call_or_heartbeat() {
        let dispatcher = Arc::new(OpeningTrustSyncDispatcher::new());
        let (up_tx, _up_rx) = mpsc::channel(4);
        let scheduler =
            SessionDownFrameScheduler::spawn(Arc::clone(&dispatcher), SessionUpSender::new(up_tx));

        scheduler
            .route(dispatch_call(41))
            .await
            .expect("first call enters Opening");
        for sequence in 0_u8..33 {
            tokio::time::timeout(
                Duration::from_millis(100),
                scheduler.route(bidi_input(41, sequence)),
            )
            .await
            .expect("shared frame loop never waits on one call's ingress")
            .expect("33-frame Opening burst stays inside the bounded budget");
        }
        dispatcher.wait_for_input_start().await;

        scheduler
            .route(request_result())
            .await
            .expect("trust-sync RequestResult bypasses the data lane");
        scheduler
            .route(dispatch_call(52))
            .await
            .expect("independent call bypasses the blocked data lane");
        scheduler
            .route(heartbeat())
            .await
            .expect("heartbeat bypasses the blocked data lane");

        assert_eq!(dispatcher.request_results.load(Ordering::SeqCst), 1);
        assert_eq!(
            *dispatcher
                .opened_calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![41, 52]
        );
        assert_eq!(dispatcher.heartbeats.load(Ordering::SeqCst), 1);
        assert!(
            dispatcher
                .inputs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "Opening remains the input-delivery barrier"
        );

        dispatcher.input_gate.send_replace(true);
        dispatcher.wait_for_all_inputs(33).await;
        assert_eq!(
            *dispatcher
                .inputs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            (0_u8..33).collect::<Vec<_>>(),
            "one call's scheduled frames remain exactly-once FIFO"
        );
    }

    #[tokio::test]
    async fn one_call_cannot_exceed_its_bounded_scheduler_budget() {
        let dispatcher = Arc::new(OpeningTrustSyncDispatcher::new());
        let (up_tx, _up_rx) = mpsc::channel(1);
        let scheduler = SessionDownFrameScheduler::spawn(dispatcher, SessionUpSender::new(up_tx));

        for sequence in 0..BIDI_FRAMES_PER_CALL {
            scheduler
                .route(bidi_input(61, sequence as u8))
                .await
                .expect("frame inside per-call budget");
        }
        let error = scheduler
            .route(bidi_input(61, 255))
            .await
            .expect_err("the next frame must fail the carrier explicitly");
        assert!(matches!(
            error,
            SessionFrameScheduleError::CallSaturated {
                call_id: 61,
                limit: BIDI_FRAMES_PER_CALL,
            }
        ));
    }

    #[test]
    fn request_result_is_not_classified_as_bidi_data() {
        assert_eq!(bidi_input_call_id(&request_result()), None);
        assert_eq!(bidi_input_call_id(&bidi_input(7, 1)), Some(7));
    }
}
