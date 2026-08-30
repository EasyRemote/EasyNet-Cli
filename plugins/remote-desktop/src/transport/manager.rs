// EasyNet CLI — remote desktop transport manager
// ===============================================
//
// File: plugins/remote-desktop/src/transport/manager.rs
// Description: Epoch-scoped endpoint and owned media-task lifecycle.
//
// Protocol Responsibility:
// - None. Axon does not own product media endpoints.
//
// Implementation Approach:
// - Allocate monotonic epochs, expose cloneable endpoint access, and retain the
//   only stop/completion handles in a managed endpoint.
//
// Usage Contract:
// - Callbacks and candidate application must compare the endpoint epoch before
//   mutating session state. Replacement retires and settles the old generation.
//
// Architectural Position:
// - Remote-desktop plugin transport-resource owner.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock, RwLockReadGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::runtime::Handle;
use tokio::sync::watch;
use webrtc::peer_connection::PeerConnection;

use crate::daemon::plugins::remote_desktop::session_transport_state::{
    PreviewTransportEpoch, TransportEpoch,
};

static TRANSPORT_RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
static TRANSPORT_CLEANUP_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn transport_runtime_handle() -> anyhow::Result<Handle> {
    let runtime = TRANSPORT_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("easynet-webrtc-runtime")
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
    });
    match runtime {
        Ok(runtime) => Ok(runtime.handle().clone()),
        Err(error) => anyhow::bail!("build RemoteApp WebRTC runtime: {error}"),
    }
}

fn transport_cleanup_runtime_handle() -> Handle {
    TRANSPORT_CLEANUP_RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build RemoteApp settlement cleanup runtime")
        })
        .handle()
        .clone()
}

pub(in crate::daemon::plugins::remote_desktop) const TRANSPORT_SETTLEMENT_DEADLINE: Duration =
    Duration::from_secs(3);
const TRANSPORT_SETTLEMENT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const TRANSPORT_SETTLEMENT_EXECUTOR_SLICE: Duration = Duration::from_millis(50);

#[derive(Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct DirectWebRtcEndpoint {
    pub(in crate::daemon::plugins::remote_desktop) epoch: TransportEpoch,
    pub(in crate::daemon::plugins::remote_desktop) peer_connection: Arc<dyn PeerConnection>,
}

struct ManagedDirectWebRtcEndpoint {
    access: DirectWebRtcEndpoint,
    stop_tx: watch::Sender<bool>,
    completion: Option<thread::JoinHandle<()>>,
}

struct ManagedDirectWebRtcReservation {
    stop_tx: watch::Sender<bool>,
    completion: Receiver<DirectWebRtcReservationCompletion>,
}

#[derive(Debug, Clone, Copy)]
struct DirectWebRtcReservationCompletion {
    settled: bool,
}

enum DirectWebRtcEndpointCommit {
    Accepted,
    Rejected { settled: bool },
}

impl DirectWebRtcEndpointCommit {
    fn accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }

    fn settled(&self) -> bool {
        match self {
            Self::Accepted => true,
            Self::Rejected { settled } => *settled,
        }
    }
}

#[derive(Default)]
struct DirectWebRtcAdmissionState {
    sealed: bool,
    high_watermark: Option<TransportEpoch>,
    pending: HashMap<TransportEpoch, ManagedDirectWebRtcReservation>,
}

struct ManagedDiagnosticPreview {
    epoch: PreviewTransportEpoch,
    stop_tx: watch::Sender<bool>,
    completion: Receiver<PreviewTaskGroupCompletion>,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::daemon::plugins::remote_desktop) struct PreviewTaskGroupCompletion;

/// Result of one bounded transport-settlement observation.
///
/// `Pending` is deliberately distinct from `Failed`: a deadline is not proof
/// that a worker leaked or failed, and the owner must retain every completion
/// handle so a later observation can still prove settlement. `Failed` is
/// reserved for an explicit negative receipt, a disconnected receipt channel,
/// or a panicked worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum TransportSettlementStatus {
    Settled,
    Pending,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum TransportSettlementFailureKind {
    ExplicitFailure,
    Panicked,
    ExecutorUnavailable,
}

impl TransportSettlementFailureKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitFailure => "explicit_failure",
            Self::Panicked => "panicked",
            Self::ExecutorUnavailable => "executor_unavailable",
        }
    }
}

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct TransportSettlementJobContext {
    pub(in crate::daemon::plugins::remote_desktop) job_kind: &'static str,
    pub(in crate::daemon::plugins::remote_desktop) session_id: Option<String>,
}

impl TransportSettlementJobContext {
    fn anonymous() -> Self {
        Self {
            job_kind: "anonymous",
            session_id: None,
        }
    }
}

pub(in crate::daemon::plugins::remote_desktop) trait TransportSettlementJob:
    Send
{
    fn settlement_status_until(&mut self, deadline: Instant) -> TransportSettlementStatus;

    /// Earliest useful time for another observation after returning Pending.
    /// Jobs without an application-level backoff use the executor poll cadence.
    fn next_poll_at(&self) -> Option<Instant> {
        None
    }

    fn context(&self) -> TransportSettlementJobContext {
        TransportSettlementJobContext::anonymous()
    }

    /// Project an explicit operational outcome for a job that can no longer
    /// prove transport settlement. The resource owner remains quarantined even
    /// after this succeeds; this hook exists to keep the product session from
    /// remaining anonymously stuck in Closing.
    fn project_quarantine(
        &mut self,
        _failure: TransportSettlementFailureKind,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

struct QuarantinedTransportSettlementJob {
    job: Mutex<Box<dyn TransportSettlementJob>>,
    context: TransportSettlementJobContext,
    failure: TransportSettlementFailureKind,
    first_failed_at_ms: u64,
    projection: Mutex<QuarantineProjectionState>,
}

struct QuarantineProjectionState {
    projection_complete: bool,
    projection_attempts: u32,
    next_projection_attempt: Instant,
    projection_retry_delay: Duration,
    last_projection_error: Option<String>,
}

type SharedQuarantinedTransportSettlementJob = Arc<QuarantinedTransportSettlementJob>;

impl QuarantinedTransportSettlementJob {
    fn new(job: Box<dyn TransportSettlementJob>, failure: TransportSettlementFailureKind) -> Self {
        let context = job.context();
        Self {
            job: Mutex::new(job),
            context,
            failure,
            first_failed_at_ms: wall_clock_ms(),
            projection: Mutex::new(QuarantineProjectionState {
                projection_complete: false,
                projection_attempts: 0,
                next_projection_attempt: Instant::now(),
                projection_retry_delay: Duration::from_millis(100),
                last_projection_error: None,
            }),
        }
    }

    fn retry_projection(&self, now: Instant) {
        {
            let mut projection = match self.projection.lock() {
                Ok(projection) => projection,
                Err(poisoned) => poisoned.into_inner(),
            };
            if projection.projection_complete || now < projection.next_projection_attempt {
                return;
            }
            projection.projection_attempts = projection.projection_attempts.saturating_add(1);
        }
        let mut job = match self.job.lock() {
            Ok(job) => job,
            Err(poisoned) => poisoned.into_inner(),
        };
        match catch_unwind(AssertUnwindSafe(|| job.project_quarantine(self.failure))) {
            Ok(Ok(())) => {
                let mut projection = match self.projection.lock() {
                    Ok(projection) => projection,
                    Err(poisoned) => poisoned.into_inner(),
                };
                projection.projection_complete = true;
                projection.last_projection_error = None;
            }
            Ok(Err(error)) => {
                self.defer_projection_retry(error.to_string());
            }
            Err(_) => {
                self.defer_projection_retry("quarantine outcome projection panicked".to_string());
            }
        }
    }

    fn defer_projection_retry(&self, error: String) {
        let mut projection = match self.projection.lock() {
            Ok(projection) => projection,
            Err(poisoned) => poisoned.into_inner(),
        };
        projection.last_projection_error = Some(error);
        projection.next_projection_attempt = Instant::now() + projection.projection_retry_delay;
        projection.projection_retry_delay =
            (projection.projection_retry_delay * 2).min(Duration::from_secs(5));
    }

    fn health_failure(&self) -> TransportSettlementHealthFailure {
        let projection = match self.projection.lock() {
            Ok(projection) => projection,
            Err(poisoned) => poisoned.into_inner(),
        };
        TransportSettlementHealthFailure {
            job_kind: self.context.job_kind,
            session_id: self.context.session_id.clone(),
            failure: self.failure,
            first_failed_at_ms: self.first_failed_at_ms,
            projection_complete: projection.projection_complete,
            projection_attempts: projection.projection_attempts,
            last_projection_error: projection.last_projection_error.clone(),
        }
    }

    fn next_projection_attempt(&self) -> Option<Instant> {
        let projection = match self.projection.lock() {
            Ok(projection) => projection,
            Err(poisoned) => poisoned.into_inner(),
        };
        (!projection.projection_complete).then_some(projection.next_projection_attempt)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct TransportSettlementHealth {
    quarantined_jobs: usize,
    pending_outcome_projections: usize,
    oldest_failure_at_ms: Option<u64>,
    failures: Vec<TransportSettlementHealthFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TransportSettlementHealthFailure {
    job_kind: &'static str,
    session_id: Option<String>,
    failure: TransportSettlementFailureKind,
    first_failed_at_ms: u64,
    projection_complete: bool,
    projection_attempts: u32,
    last_projection_error: Option<String>,
}

impl TransportSettlementHealth {
    pub(in crate::daemon::plugins::remote_desktop) fn admission_open(&self) -> bool {
        self.quarantined_jobs == 0
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn quarantined_jobs(&self) -> usize {
        self.quarantined_jobs
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "status": if self.admission_open() { "healthy" } else { "unhealthy" },
            "admission_open": self.admission_open(),
            "quarantined_jobs": self.quarantined_jobs,
            "pending_outcome_projections": self.pending_outcome_projections,
            "oldest_failure_at_ms": self.oldest_failure_at_ms,
            "failures": self.failures.iter().map(|failure| json!({
                "job_kind": failure.job_kind,
                "session_id": failure.session_id,
                "failure": failure.failure.as_str(),
                "first_failed_at_ms": failure.first_failed_at_ms,
                "projection_complete": failure.projection_complete,
                "projection_attempts": failure.projection_attempts,
                "last_projection_error": failure.last_projection_error,
            })).collect::<Vec<_>>(),
        })
    }
}

/// Linearization boundary shared by session creation and transport quarantine.
///
/// A read permit acquired while the owner count is zero may finish one bounded
/// session-map insertion. Quarantine first increments the monotonic owner count,
/// then its projector crosses the write barrier before publishing any session
/// outcome. Consequently an admission is ordered either wholly before the first
/// quarantine owner or wholly after it; a health snapshot is never used as the
/// concurrency primitive.
#[derive(Default)]
struct SettlementAdmissionGate {
    barrier: RwLock<()>,
    quarantined_owners: AtomicU64,
}

impl SettlementAdmissionGate {
    fn acquire(&self) -> Result<TransportSettlementAdmissionPermit<'_>, usize> {
        let barrier = match self.barrier.read() {
            Ok(barrier) => barrier,
            Err(poisoned) => poisoned.into_inner(),
        };
        let quarantined_owners = self.quarantined_owner_count();
        if quarantined_owners != 0 {
            drop(barrier);
            return Err(quarantined_owners);
        }
        Ok(TransportSettlementAdmissionPermit { _barrier: barrier })
    }

    fn announce_quarantined_owner(&self) {
        let _ =
            self.quarantined_owners
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    Some(count.saturating_add(1))
                });
    }

    fn quarantined_owner_count(&self) -> usize {
        self.quarantined_owners
            .load(Ordering::Acquire)
            .try_into()
            .unwrap_or(usize::MAX)
    }

    fn synchronize_announced_admissions(&self) {
        let barrier = match self.barrier.write() {
            Ok(barrier) => barrier,
            Err(poisoned) => poisoned.into_inner(),
        };
        drop(barrier);
    }
}

pub(in crate::daemon::plugins::remote_desktop) struct TransportSettlementAdmissionPermit<'a> {
    _barrier: RwLockReadGuard<'a, ()>,
}

#[derive(Clone)]
struct TransportSettlementQuarantine {
    records: Arc<Mutex<Vec<SharedQuarantinedTransportSettlementJob>>>,
    admission_gate: Arc<SettlementAdmissionGate>,
    wake_tx: Sender<()>,
}

impl TransportSettlementQuarantine {
    fn retain(
        &self,
        job: Box<dyn TransportSettlementJob>,
        failure: TransportSettlementFailureKind,
    ) {
        let record = Arc::new(QuarantinedTransportSettlementJob::new(job, failure));
        let mut records = match self.records.lock() {
            Ok(records) => records,
            Err(poisoned) => poisoned.into_inner(),
        };
        // This atomic announcement is the fail-closed linearization point. It
        // is deliberately safe on a submitter that still owns the session-map
        // mutex; no session/recovery callback runs on this path. Publishing it
        // while the health-visible record collection is locked also prevents a
        // health reader from observing a count without its typed owner record.
        self.admission_gate.announce_quarantined_owner();
        records.push(record);
        drop(records);
        if self.wake_tx.send(()).is_err() {
            eprintln!(
                "[remote-desktop] quarantine projector unavailable; ownership remains retained and admission remains closed"
            );
        }
    }
}

/// Cloneable submission handle for the one process-owned transport settlement
/// executor. The executor is created with the manager; terminal paths enqueue
/// ownership instead of spawning ad-hoc reaper threads.
#[derive(Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct TransportSettlementQueue {
    tx: Sender<Box<dyn TransportSettlementJob>>,
    quarantine: TransportSettlementQuarantine,
    cleanup_runtime: Handle,
}

impl TransportSettlementQueue {
    fn start() -> Self {
        let (tx, rx) = mpsc::channel::<Box<dyn TransportSettlementJob>>();
        let (quarantine_wake_tx, quarantine_wake_rx) = mpsc::channel();
        let quarantine_records = Arc::new(Mutex::new(Vec::new()));
        let admission_gate = Arc::new(SettlementAdmissionGate::default());
        let quarantine = TransportSettlementQuarantine {
            records: Arc::clone(&quarantine_records),
            admission_gate: Arc::clone(&admission_gate),
            wake_tx: quarantine_wake_tx,
        };
        let cleanup_runtime = transport_cleanup_runtime_handle();
        let projector_records = Arc::clone(&quarantine_records);
        let projector_admission_gate = Arc::clone(&admission_gate);
        thread::Builder::new()
            .name("easynet-rd-quarantine-projector".into())
            .spawn(move || {
                run_transport_quarantine_projector(
                    quarantine_wake_rx,
                    projector_records,
                    projector_admission_gate,
                )
            })
            .expect("start RemoteApp transport quarantine projector");
        let worker_quarantine = quarantine.clone();
        thread::Builder::new()
            .name("easynet-rd-settlement-executor".into())
            .spawn(move || run_transport_settlement_executor(rx, worker_quarantine))
            .expect("start RemoteApp transport settlement executor");
        Self {
            tx,
            quarantine,
            cleanup_runtime,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn enqueue<J>(&self, job: J)
    where
        J: TransportSettlementJob + 'static,
    {
        self.enqueue_boxed(Box::new(job));
    }

    fn enqueue_boxed(&self, job: Box<dyn TransportSettlementJob>) {
        if let Err(error) = self.tx.send(job) {
            eprintln!(
                "[remote-desktop] settlement executor unavailable; retaining ownership in quarantine"
            );
            self.quarantine
                .retain(error.0, TransportSettlementFailureKind::ExecutorUnavailable);
        }
    }

    fn acquire_admission(&self) -> Result<TransportSettlementAdmissionPermit<'_>, usize> {
        self.quarantine.admission_gate.acquire()
    }

    fn cleanup_runtime(&self) -> Handle {
        self.cleanup_runtime.clone()
    }

    #[cfg(test)]
    fn with_disconnected_executor_for_test() -> Self {
        let (tx, rx) = mpsc::channel::<Box<dyn TransportSettlementJob>>();
        drop(rx);
        let (quarantine_wake_tx, quarantine_wake_rx) = mpsc::channel();
        let records = Arc::new(Mutex::new(Vec::new()));
        let admission_gate = Arc::new(SettlementAdmissionGate::default());
        let quarantine = TransportSettlementQuarantine {
            records: Arc::clone(&records),
            admission_gate: Arc::clone(&admission_gate),
            wake_tx: quarantine_wake_tx,
        };
        thread::Builder::new()
            .name("easynet-rd-test-quarantine-projector".into())
            .spawn(move || {
                run_transport_quarantine_projector(quarantine_wake_rx, records, admission_gate)
            })
            .expect("start test RemoteApp transport quarantine projector");
        let cleanup_runtime = transport_cleanup_runtime_handle();
        Self {
            tx,
            quarantine,
            cleanup_runtime,
        }
    }

    #[cfg(test)]
    fn quarantined_job_count(&self) -> usize {
        match self.quarantine.records.lock() {
            Ok(records) => records.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    fn health(&self) -> TransportSettlementHealth {
        let (records, quarantined_jobs) = match self.quarantine.records.lock() {
            Ok(records) => (
                records.clone(),
                self.quarantine.admission_gate.quarantined_owner_count(),
            ),
            Err(poisoned) => (
                poisoned.into_inner().clone(),
                self.quarantine.admission_gate.quarantined_owner_count(),
            ),
        };
        let failures = records
            .iter()
            .map(|record| record.health_failure())
            .collect::<Vec<_>>();
        TransportSettlementHealth {
            quarantined_jobs,
            pending_outcome_projections: failures
                .iter()
                .filter(|failure| !failure.projection_complete)
                .count(),
            oldest_failure_at_ms: failures
                .iter()
                .map(|failure| failure.first_failed_at_ms)
                .min(),
            failures,
        }
    }
}

fn run_transport_settlement_executor(
    rx: Receiver<Box<dyn TransportSettlementJob>>,
    quarantine: TransportSettlementQuarantine,
) {
    let mut pending = VecDeque::new();
    let mut delayed = Vec::<(Instant, Box<dyn TransportSettlementJob>)>::new();
    let mut disconnected = false;
    loop {
        let now = Instant::now();
        let mut index = 0;
        while index < delayed.len() {
            if delayed[index].0 <= now {
                let (_, job) = delayed.swap_remove(index);
                pending.push_back(job);
            } else {
                index += 1;
            }
        }
        if pending.is_empty() && !disconnected {
            let delayed_wait = delayed
                .iter()
                .map(|(ready_at, _)| ready_at.saturating_duration_since(now))
                .min();
            let wait = delayed_wait;
            if let Some(wait) = wait {
                match rx.recv_timeout(wait.max(Duration::from_millis(1))) {
                    Ok(job) => pending.push_back(job),
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => disconnected = true,
                }
            } else {
                match rx.recv() {
                    Ok(job) => pending.push_back(job),
                    Err(_) => disconnected = true,
                }
            }
        }
        while !disconnected {
            match rx.try_recv() {
                Ok(job) => pending.push_back(job),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        let Some(mut job) = pending.pop_front() else {
            if disconnected {
                if delayed.is_empty() {
                    return;
                }
                let delayed_wait = delayed
                    .iter()
                    .map(|(ready_at, _)| ready_at.saturating_duration_since(Instant::now()))
                    .min()
                    .unwrap_or(TRANSPORT_SETTLEMENT_EXECUTOR_SLICE);
                thread::park_timeout(delayed_wait);
            }
            continue;
        };
        let status = catch_unwind(AssertUnwindSafe(|| {
            job.settlement_status_until(Instant::now() + TRANSPORT_SETTLEMENT_EXECUTOR_SLICE)
        }));
        match status {
            Ok(TransportSettlementStatus::Pending) => {
                if let Some(ready_at) = job
                    .next_poll_at()
                    .filter(|ready_at| *ready_at > Instant::now())
                {
                    delayed.push((ready_at, job));
                } else {
                    pending.push_back(job);
                    thread::sleep(TRANSPORT_SETTLEMENT_POLL_INTERVAL);
                }
            }
            Ok(TransportSettlementStatus::Settled) => {}
            Ok(TransportSettlementStatus::Failed) => {
                eprintln!(
                    "[remote-desktop] settlement job reported an explicit failure; retaining it in quarantine"
                );
                quarantine.retain(job, TransportSettlementFailureKind::ExplicitFailure);
            }
            Err(_) => {
                eprintln!(
                    "[remote-desktop] settlement job panicked; retaining ownership in quarantine"
                );
                quarantine.retain(job, TransportSettlementFailureKind::Panicked);
            }
        }
    }
}

fn retry_quarantine_projections(records: &Mutex<Vec<SharedQuarantinedTransportSettlementJob>>) {
    let records = match records.lock() {
        Ok(records) => records.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    let now = Instant::now();
    for record in records {
        record.retry_projection(now);
    }
}

fn next_quarantine_projection_at(
    records: &Mutex<Vec<SharedQuarantinedTransportSettlementJob>>,
) -> Option<Instant> {
    let records = match records.lock() {
        Ok(records) => records.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    records
        .iter()
        .filter_map(|record| record.next_projection_attempt())
        .min()
}

fn quarantine_records_are_empty(
    records: &Mutex<Vec<SharedQuarantinedTransportSettlementJob>>,
) -> bool {
    match records.lock() {
        Ok(records) => records.is_empty(),
        Err(poisoned) => poisoned.into_inner().is_empty(),
    }
}

fn run_transport_quarantine_projector(
    wake_rx: Receiver<()>,
    records: Arc<Mutex<Vec<SharedQuarantinedTransportSettlementJob>>>,
    admission_gate: Arc<SettlementAdmissionGate>,
) {
    let mut admission_synchronized = false;
    loop {
        if !admission_synchronized && admission_gate.quarantined_owner_count() != 0 {
            admission_gate.synchronize_announced_admissions();
            admission_synchronized = true;
        }
        retry_quarantine_projections(&records);
        let wake = match next_quarantine_projection_at(&records) {
            Some(ready_at) => wake_rx.recv_timeout(
                ready_at
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(1)),
            ),
            None => match wake_rx.recv() {
                Ok(()) => Ok(()),
                Err(_) => {
                    if quarantine_records_are_empty(&records) {
                        return;
                    }
                    // Completed quarantine records retain failed resource
                    // ownership for the process lifetime. With every producer
                    // gone there is no future work, so park this owner forever
                    // instead of dropping the records and fabricating cleanup.
                    thread::park();
                    continue;
                }
            },
        };
        match wake {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if quarantine_records_are_empty(&records) {
                    return;
                }
                thread::park();
            }
        }
    }
}

pub(in crate::daemon::plugins::remote_desktop) struct RetiredDiagnosticPreview {
    completion: Option<Receiver<PreviewTaskGroupCompletion>>,
    failed: bool,
    settlement_queue: Option<TransportSettlementQueue>,
}

impl RetiredDiagnosticPreview {
    /// Wait until the complete diagnostic task group (control, forwarding,
    /// capture, and any blocking H.264 worker) has exited.
    pub(in crate::daemon::plugins::remote_desktop) fn settle(mut self) -> bool {
        match self.settlement_status_until(Instant::now() + TRANSPORT_SETTLEMENT_DEADLINE) {
            TransportSettlementStatus::Settled => true,
            TransportSettlementStatus::Pending => {
                self.enqueue_for_settlement();
                false
            }
            TransportSettlementStatus::Failed => false,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn settlement_status_until(
        &mut self,
        deadline: Instant,
    ) -> TransportSettlementStatus {
        if self.failed {
            return TransportSettlementStatus::Failed;
        }
        if let Some(completion) = self.completion.as_ref() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match completion.recv_timeout(remaining) {
                Ok(PreviewTaskGroupCompletion) => {
                    self.completion = None;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    eprintln!(
                        "[remote-desktop] diagnostic preview completion channel disconnected before worker-group receipt"
                    );
                    self.completion = None;
                    self.failed = true;
                    return TransportSettlementStatus::Failed;
                }
                Err(RecvTimeoutError::Timeout) => {
                    eprintln!(
                        "[remote-desktop] diagnostic preview settlement exceeded its bounded deadline"
                    );
                    return TransportSettlementStatus::Pending;
                }
            }
        }
        TransportSettlementStatus::Settled
    }

    fn enqueue_for_settlement(mut self) {
        if self.completion.is_none() || self.failed {
            return;
        }
        let queue = self
            .settlement_queue
            .take()
            .expect("managed diagnostic preview retains settlement queue");
        queue.enqueue(self);
    }
}

impl TransportSettlementJob for RetiredDiagnosticPreview {
    fn settlement_status_until(&mut self, deadline: Instant) -> TransportSettlementStatus {
        RetiredDiagnosticPreview::settlement_status_until(self, deadline)
    }

    fn context(&self) -> TransportSettlementJobContext {
        TransportSettlementJobContext {
            job_kind: "diagnostic_preview",
            session_id: None,
        }
    }
}

impl ManagedDiagnosticPreview {
    fn signal_stop(self, settlement_queue: TransportSettlementQueue) -> RetiredDiagnosticPreview {
        let _ = self.stop_tx.send(true);
        RetiredDiagnosticPreview {
            completion: Some(self.completion),
            failed: false,
            settlement_queue: Some(settlement_queue),
        }
    }

    fn retire(self, settlement_queue: TransportSettlementQueue) {
        self.signal_stop(settlement_queue).enqueue_for_settlement();
    }
}

pub(in crate::daemon::plugins::remote_desktop) struct RetiredDirectWebRtcEndpoint {
    completion: Option<thread::JoinHandle<()>>,
    pending: VecDeque<Receiver<DirectWebRtcReservationCompletion>>,
    failed: bool,
    settlement_queue: Option<TransportSettlementQueue>,
}

impl RetiredDirectWebRtcEndpoint {
    pub(in crate::daemon::plugins::remote_desktop) fn settle(mut self) -> bool {
        match self.settlement_status_until(Instant::now() + TRANSPORT_SETTLEMENT_DEADLINE) {
            TransportSettlementStatus::Settled => true,
            TransportSettlementStatus::Pending => {
                self.enqueue_for_settlement();
                false
            }
            TransportSettlementStatus::Failed => false,
        }
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn settle_until(
        &mut self,
        deadline: Instant,
    ) -> bool {
        self.settlement_status_until(deadline) == TransportSettlementStatus::Settled
    }

    pub(in crate::daemon::plugins::remote_desktop) fn settlement_status_until(
        &mut self,
        deadline: Instant,
    ) -> TransportSettlementStatus {
        while let Some(completion) = self.pending.front() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match completion.recv_timeout(remaining) {
                Ok(DirectWebRtcReservationCompletion { settled: true }) => {
                    self.pending.pop_front();
                }
                Ok(DirectWebRtcReservationCompletion { settled: false }) => {
                    eprintln!("[remote-desktop] rejected pending WebRTC endpoint did not settle");
                    self.pending.pop_front();
                    self.failed = true;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    eprintln!(
                        "[remote-desktop] direct WebRTC reservation completion channel disconnected"
                    );
                    self.pending.pop_front();
                    self.failed = true;
                }
                Err(RecvTimeoutError::Timeout) => {
                    eprintln!(
                        "[remote-desktop] pending direct WebRTC settlement exceeded its bounded deadline"
                    );
                    return TransportSettlementStatus::Pending;
                }
            }
        }
        let Some(completion) = self.completion.as_ref() else {
            return if self.failed {
                TransportSettlementStatus::Failed
            } else {
                TransportSettlementStatus::Settled
            };
        };
        while !completion.is_finished() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                eprintln!(
                    "[remote-desktop] direct WebRTC settlement exceeded its bounded deadline"
                );
                return TransportSettlementStatus::Pending;
            }
            thread::sleep(remaining.min(TRANSPORT_SETTLEMENT_POLL_INTERVAL));
        }
        let completion = self
            .completion
            .take()
            .expect("finished direct WebRTC worker retains join ownership");
        if completion.join().is_err() {
            self.failed = true;
        }
        if self.failed {
            TransportSettlementStatus::Failed
        } else {
            TransportSettlementStatus::Settled
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn enqueue_for_settlement(mut self) {
        let queue = self
            .settlement_queue
            .take()
            .expect("managed direct WebRTC endpoint retains settlement queue");
        queue.enqueue(self);
    }
}

impl TransportSettlementJob for RetiredDirectWebRtcEndpoint {
    fn settlement_status_until(&mut self, deadline: Instant) -> TransportSettlementStatus {
        RetiredDirectWebRtcEndpoint::settlement_status_until(self, deadline)
    }

    fn context(&self) -> TransportSettlementJobContext {
        TransportSettlementJobContext {
            job_kind: "direct_webrtc_endpoint",
            session_id: None,
        }
    }
}

impl ManagedDirectWebRtcEndpoint {
    fn signal_stop(
        mut self,
        settlement_queue: TransportSettlementQueue,
    ) -> RetiredDirectWebRtcEndpoint {
        let _ = self.stop_tx.send(true);
        RetiredDirectWebRtcEndpoint {
            completion: self.completion.take(),
            pending: VecDeque::new(),
            failed: false,
            settlement_queue: Some(settlement_queue),
        }
    }

    fn retire(self, settlement_queue: TransportSettlementQueue) {
        self.signal_stop(settlement_queue).enqueue_for_settlement();
    }

    fn stop_and_settle(self, settlement_queue: TransportSettlementQueue) {
        let retired = self.signal_stop(settlement_queue);
        if !retired.settle() {
            // `settle` transfers Pending ownership to the executor. An
            // explicit Failed status has no remaining receiver/join ownership.
        }
    }
}

/// Admission token for one direct WebRTC generation being built outside the
/// session-store lock. The manager retains its cancel/completion peer so a
/// terminal transition can seal the session and settle this pending work.
pub(in crate::daemon::plugins::remote_desktop) struct DirectWebRtcEndpointReservation {
    manager: Arc<RemoteDesktopTransportManager>,
    session_id: String,
    epoch: TransportEpoch,
    stop_tx: watch::Sender<bool>,
    stop_rx: watch::Receiver<bool>,
    completion_tx: Option<Sender<DirectWebRtcReservationCompletion>>,
}

impl DirectWebRtcEndpointReservation {
    pub(in crate::daemon::plugins::remote_desktop) fn stop_receiver(
        &self,
    ) -> watch::Receiver<bool> {
        self.stop_rx.clone()
    }

    /// Atomically transition this reservation into the active endpoint slot.
    /// A terminal seal wins the race and forces the candidate to settle before
    /// the pending reservation can report completion.
    pub(in crate::daemon::plugins::remote_desktop) fn commit(
        mut self,
        endpoint: DirectWebRtcEndpoint,
        completion: thread::JoinHandle<()>,
    ) -> bool {
        let outcome = self.manager.commit_reserved_endpoint(
            &self.session_id,
            self.epoch,
            endpoint,
            self.stop_tx.clone(),
            completion,
        );
        if let Some(completion_tx) = self.completion_tx.take() {
            let _ = completion_tx.send(DirectWebRtcReservationCompletion {
                settled: outcome.settled(),
            });
        }
        outcome.accepted()
    }

    /// Resolve a setup attempt that never allocated a peer connection, or one
    /// whose partially-created peer has already produced a real close receipt.
    pub(in crate::daemon::plugins::remote_desktop) fn complete_without_endpoint(mut self) {
        self.manager
            .cancel_endpoint_reservation(&self.session_id, self.epoch);
        if let Some(completion_tx) = self.completion_tx.take() {
            let _ = completion_tx.send(DirectWebRtcReservationCompletion { settled: true });
        }
    }

    /// Transfer a partially-created peer connection into a dedicated cleanup
    /// owner. A bounded close timeout is only `Pending`; this reaper retains the
    /// peer and retries until the WebRTC implementation returns a real close
    /// receipt. Terminal settlement observes the same reservation completion
    /// channel and therefore cannot manufacture success from a timeout.
    pub(in crate::daemon::plugins::remote_desktop) fn complete_with_endpoint_cleanup(
        self,
        peer_connection: Arc<dyn PeerConnection>,
    ) {
        let runtime = self.manager.settlement_queue().cleanup_runtime();
        self.complete_with_cleanup_owner(move |deadline| {
            runtime.block_on(super::webrtc_media::close_peer_connection_until(
                &peer_connection,
                deadline,
            ))
        });
    }

    fn complete_with_cleanup_owner<F>(mut self, mut cleanup: F)
    where
        F: FnMut(Duration) -> bool + Send + 'static,
    {
        let Some(completion_tx) = self.completion_tx.take() else {
            return;
        };
        let job = EndpointSetupCleanupJob {
            cleanup: move |deadline| cleanup(deadline),
            completion_tx: Some(completion_tx),
            manager: Arc::downgrade(&self.manager),
            session_id: self.session_id.clone(),
            epoch: self.epoch,
        };
        self.manager.settlement_queue().enqueue(job);
    }
}

struct EndpointSetupCleanupJob<F>
where
    F: FnMut(Duration) -> bool + Send + 'static,
{
    cleanup: F,
    completion_tx: Option<Sender<DirectWebRtcReservationCompletion>>,
    manager: std::sync::Weak<RemoteDesktopTransportManager>,
    session_id: String,
    epoch: TransportEpoch,
}

impl<F> TransportSettlementJob for EndpointSetupCleanupJob<F>
where
    F: FnMut(Duration) -> bool + Send + 'static,
{
    fn settlement_status_until(&mut self, deadline: Instant) -> TransportSettlementStatus {
        if !(self.cleanup)(deadline.saturating_duration_since(Instant::now())) {
            return TransportSettlementStatus::Pending;
        }
        if let Some(completion_tx) = self.completion_tx.take() {
            let _ = completion_tx.send(DirectWebRtcReservationCompletion { settled: true });
        }
        // Publish the real receipt before removing the manager-side receiver.
        // A concurrent terminal sweep either drains and observes it or runs
        // after cleanup is complete; it cannot miss an unsettled peer.
        if let Some(manager) = self.manager.upgrade() {
            manager.cancel_endpoint_reservation(&self.session_id, self.epoch);
        }
        TransportSettlementStatus::Settled
    }

    fn context(&self) -> TransportSettlementJobContext {
        TransportSettlementJobContext {
            job_kind: "endpoint_setup_cleanup",
            session_id: Some(self.session_id.clone()),
        }
    }

    fn project_quarantine(
        &mut self,
        _failure: TransportSettlementFailureKind,
    ) -> anyhow::Result<()> {
        // The cleanup owner remains retained in quarantine, so this is not a
        // cleanup receipt. It is an explicit negative receipt that lets the
        // parent terminal settlement leave Closing and publish durable Failed.
        // Taking the sender makes the projection exactly-once across retries.
        if let Some(completion_tx) = self.completion_tx.take() {
            let _ = completion_tx.send(DirectWebRtcReservationCompletion { settled: false });
        }
        Ok(())
    }
}

impl Drop for DirectWebRtcEndpointReservation {
    fn drop(&mut self) {
        let Some(completion_tx) = self.completion_tx.take() else {
            return;
        };
        // Do not remove the manager-side receiver here. An unwind may occur
        // after a peer was allocated but before cleanup ownership transferred;
        // terminal teardown must still observe the explicit negative receipt.
        let _ = completion_tx.send(DirectWebRtcReservationCompletion { settled: false });
    }
}

pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopTransportManager {
    endpoints: Mutex<HashMap<String, ManagedDirectWebRtcEndpoint>>,
    direct_admission: Mutex<HashMap<String, DirectWebRtcAdmissionState>>,
    previews: Mutex<HashMap<String, ManagedDiagnosticPreview>>,
    settlement_queue: TransportSettlementQueue,
    next_epoch: AtomicU64,
}

impl RemoteDesktopTransportManager {
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self {
            endpoints: Mutex::new(HashMap::new()),
            direct_admission: Mutex::new(HashMap::new()),
            previews: Mutex::new(HashMap::new()),
            settlement_queue: TransportSettlementQueue::start(),
            // Epochs are public stale-callback fences, so a daemon restart
            // must not reset the namespace to one. Recovery snapshots tighten
            // this seed further through `observe_prior_epoch`.
            next_epoch: AtomicU64::new(process_epoch_seed()),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn allocate_epoch(&self) -> TransportEpoch {
        let value = self.next_epoch.fetch_add(1, Ordering::AcqRel);
        assert_ne!(
            value,
            u64::MAX,
            "RemoteApp transport epoch namespace exhausted"
        );
        TransportEpoch::new(value)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn settlement_queue(
        &self,
    ) -> TransportSettlementQueue {
        self.settlement_queue.clone()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn settlement_health(
        &self,
    ) -> TransportSettlementHealth {
        self.settlement_queue.health()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn acquire_session_admission(
        &self,
    ) -> Result<TransportSettlementAdmissionPermit<'_>, usize> {
        self.settlement_queue.acquire_admission()
    }

    /// Move the process-local allocator past an epoch persisted by an earlier
    /// daemon process. This is idempotent and safe to call for every recovered
    /// session before the plugin accepts new offers.
    pub(in crate::daemon::plugins::remote_desktop) fn observe_prior_epoch(&self, epoch: u64) {
        let next = epoch
            .checked_add(1)
            .expect("persisted RemoteApp transport epoch namespace exhausted");
        self.next_epoch.fetch_max(next, Ordering::AcqRel);
    }

    pub(in crate::daemon::plugins::remote_desktop) fn reserve_endpoint(
        self: &Arc<Self>,
        session_id: String,
        epoch: TransportEpoch,
    ) -> anyhow::Result<DirectWebRtcEndpointReservation> {
        let (stop_tx, stop_rx) = watch::channel(false);
        let (completion_tx, completion) = mpsc::channel();
        {
            let mut admission = self.direct_admission();
            let state = admission.entry(session_id.clone()).or_default();
            if state.sealed {
                anyhow::bail!(
                    "direct WebRTC admission is sealed for terminal session {session_id:?}"
                );
            }
            if state
                .high_watermark
                .is_some_and(|high_watermark| epoch <= high_watermark)
            {
                anyhow::bail!(
                    "direct WebRTC epoch {} does not advance session {session_id:?} admission high-watermark",
                    epoch.value()
                );
            }
            state.high_watermark = Some(epoch);
            for (pending_epoch, pending) in &state.pending {
                if *pending_epoch < epoch {
                    let _ = pending.stop_tx.send(true);
                }
            }
            state.pending.insert(
                epoch,
                ManagedDirectWebRtcReservation {
                    stop_tx: stop_tx.clone(),
                    completion,
                },
            );
        }
        Ok(DirectWebRtcEndpointReservation {
            manager: Arc::clone(self),
            session_id,
            epoch,
            stop_tx,
            stop_rx,
            completion_tx: Some(completion_tx),
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn endpoint(
        &self,
        session_id: &str,
    ) -> Option<DirectWebRtcEndpoint> {
        self.endpoints()
            .get(session_id)
            .map(|managed| managed.access.clone())
    }

    /// Install one generation-scoped diagnostic preview task group. Replacing
    /// a preview retires the complete old generation instead of merely
    /// replacing its stop sender in session projection state.
    pub(in crate::daemon::plugins::remote_desktop) fn activate_preview(
        &self,
        session_id: String,
        epoch: PreviewTransportEpoch,
        stop_tx: watch::Sender<bool>,
        completion: Receiver<PreviewTaskGroupCompletion>,
    ) {
        let candidate = ManagedDiagnosticPreview {
            epoch,
            stop_tx,
            completion,
        };
        let old = {
            let mut previews = self.previews();
            if previews
                .get(&session_id)
                .is_some_and(|current| current.epoch >= epoch)
            {
                drop(previews);
                candidate.retire(self.settlement_queue());
                return;
            }
            previews.insert(session_id, candidate)
        };
        if let Some(old) = old {
            old.retire(self.settlement_queue());
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn take_preview_for_settlement(
        &self,
        session_id: &str,
    ) -> Option<RetiredDiagnosticPreview> {
        self.previews()
            .remove(session_id)
            .map(|preview| preview.signal_stop(self.settlement_queue()))
    }

    /// Remove and stop one endpoint, returning its completion ownership so a
    /// terminal lifecycle can settle it after releasing the session mutex.
    pub(in crate::daemon::plugins::remote_desktop) fn take_endpoint_for_settlement(
        &self,
        session_id: &str,
    ) -> Option<RetiredDirectWebRtcEndpoint> {
        let pending = {
            let mut admission = self.direct_admission();
            let state = admission.entry(session_id.to_string()).or_default();
            state.sealed = true;
            state
                .pending
                .drain()
                .map(|(_, pending)| {
                    let _ = pending.stop_tx.send(true);
                    pending.completion
                })
                .collect::<VecDeque<_>>()
        };
        let active = self
            .endpoints()
            .remove(session_id)
            .map(|endpoint| endpoint.signal_stop(self.settlement_queue()));
        if active.is_none() && pending.is_empty() {
            return None;
        }
        Some(RetiredDirectWebRtcEndpoint {
            completion: active.and_then(|mut active| active.completion.take()),
            pending,
            failed: false,
            settlement_queue: Some(self.settlement_queue()),
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn stop_endpoint_if_epoch(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
    ) -> bool {
        if let Some(endpoint) = self.take_endpoint_if_epoch_for_settlement(session_id, epoch) {
            endpoint.enqueue_for_settlement();
            return true;
        }
        false
    }

    pub(in crate::daemon::plugins::remote_desktop) fn take_endpoint_if_epoch_for_settlement(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
    ) -> Option<RetiredDirectWebRtcEndpoint> {
        let endpoint = {
            let mut endpoints = self.endpoints();
            if endpoints
                .get(session_id)
                .is_none_or(|endpoint| endpoint.access.epoch != epoch)
            {
                return None;
            }
            endpoints.remove(session_id)
        }?;
        Some(endpoint.signal_stop(self.settlement_queue()))
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn clear_endpoints(&self) {
        self.direct_admission().clear();
        let endpoints = std::mem::take(&mut *self.endpoints());
        for (_, endpoint) in endpoints {
            endpoint.retire(self.settlement_queue());
        }
        let previews = std::mem::take(&mut *self.previews());
        for (_, preview) in previews {
            preview.retire(self.settlement_queue());
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn block_on<F: Future>(
        &self,
        future: F,
    ) -> anyhow::Result<F::Output> {
        Ok(self.runtime_handle()?.block_on(future))
    }

    fn endpoints(&self) -> MutexGuard<'_, HashMap<String, ManagedDirectWebRtcEndpoint>> {
        match self.endpoints.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn direct_admission(&self) -> MutexGuard<'_, HashMap<String, DirectWebRtcAdmissionState>> {
        match self.direct_admission.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn commit_reserved_endpoint(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
        endpoint: DirectWebRtcEndpoint,
        stop_tx: watch::Sender<bool>,
        completion: thread::JoinHandle<()>,
    ) -> DirectWebRtcEndpointCommit {
        let candidate = ManagedDirectWebRtcEndpoint {
            access: endpoint,
            stop_tx,
            completion: Some(completion),
        };
        let outcome = {
            let mut admission = self.direct_admission();
            let accepted = admission.get_mut(session_id).is_some_and(|state| {
                let pending = state.pending.remove(&epoch).is_some();
                pending && !state.sealed && state.high_watermark == Some(epoch)
            });
            if accepted {
                Ok(self.endpoints().insert(session_id.to_string(), candidate))
            } else {
                Err(candidate)
            }
        };
        match outcome {
            Ok(old) => {
                if let Some(old) = old {
                    old.retire(self.settlement_queue());
                }
                DirectWebRtcEndpointCommit::Accepted
            }
            Err(candidate) => DirectWebRtcEndpointCommit::Rejected {
                settled: candidate.signal_stop(self.settlement_queue()).settle(),
            },
        }
    }

    fn cancel_endpoint_reservation(&self, session_id: &str, epoch: TransportEpoch) {
        if let Some(state) = self.direct_admission().get_mut(session_id) {
            state.pending.remove(&epoch);
        }
    }

    fn previews(&self) -> MutexGuard<'_, HashMap<String, ManagedDiagnosticPreview>> {
        match self.previews.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn runtime_handle(
        &self,
    ) -> anyhow::Result<Handle> {
        transport_runtime_handle()
    }
}

fn process_epoch_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros().min(u64::MAX as u128 - 1) as u64)
        .unwrap_or(1)
        .max(1)
}

fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

impl Drop for RemoteDesktopTransportManager {
    fn drop(&mut self) {
        let pending = match self.direct_admission.get_mut() {
            Ok(admission) => std::mem::take(admission),
            Err(poisoned) => std::mem::take(poisoned.into_inner()),
        };
        for (_, state) in pending {
            for (_, pending) in state.pending {
                let _ = pending.stop_tx.send(true);
                let mut retired = RetiredDirectWebRtcEndpoint {
                    completion: None,
                    pending: VecDeque::from([pending.completion]),
                    failed: false,
                    settlement_queue: Some(self.settlement_queue()),
                };
                if retired.settlement_status_until(Instant::now() + TRANSPORT_SETTLEMENT_DEADLINE)
                    == TransportSettlementStatus::Pending
                {
                    retired.enqueue_for_settlement();
                }
            }
        }
        let endpoints = match self.endpoints.get_mut() {
            Ok(endpoints) => std::mem::take(endpoints),
            Err(poisoned) => std::mem::take(poisoned.into_inner()),
        };
        for (_, endpoint) in endpoints {
            endpoint.stop_and_settle(self.settlement_queue());
        }
        let previews = match self.previews.get_mut() {
            Ok(previews) => std::mem::take(previews),
            Err(poisoned) => std::mem::take(poisoned.into_inner()),
        };
        for (_, preview) in previews {
            let _ = preview.signal_stop(self.settlement_queue()).settle();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::time::Duration;

    struct PanickingSettlementJob;

    impl TransportSettlementJob for PanickingSettlementJob {
        fn settlement_status_until(&mut self, _deadline: Instant) -> TransportSettlementStatus {
            panic!("injected settlement job panic");
        }
    }

    struct PendingCleanupRuntimeOwner {
        _runtime: Handle,
    }

    impl TransportSettlementJob for PendingCleanupRuntimeOwner {
        fn settlement_status_until(&mut self, _deadline: Instant) -> TransportSettlementStatus {
            TransportSettlementStatus::Pending
        }
    }

    struct ProjectingFailedSettlementJob {
        projected: Arc<AtomicBool>,
        session_id: String,
    }

    struct RetryOnceQuarantineProjectionJob {
        attempts: Arc<AtomicUsize>,
    }

    struct DelayedPendingSettlementJob {
        attempts: Arc<AtomicUsize>,
        ready_at: Option<Instant>,
    }

    struct BlockingQuarantineProjectionJob {
        started: Option<Sender<()>>,
        release: Receiver<()>,
    }

    impl TransportSettlementJob for BlockingQuarantineProjectionJob {
        fn settlement_status_until(&mut self, _deadline: Instant) -> TransportSettlementStatus {
            TransportSettlementStatus::Failed
        }

        fn project_quarantine(
            &mut self,
            _failure: TransportSettlementFailureKind,
        ) -> anyhow::Result<()> {
            if let Some(started) = self.started.take() {
                let _ = started.send(());
            }
            let _ = self.release.recv();
            Ok(())
        }
    }

    impl TransportSettlementJob for DelayedPendingSettlementJob {
        fn settlement_status_until(&mut self, _deadline: Instant) -> TransportSettlementStatus {
            let attempt = self.attempts.fetch_add(1, Ordering::AcqRel) + 1;
            if attempt == 1 {
                self.ready_at = Some(Instant::now() + Duration::from_millis(150));
                TransportSettlementStatus::Pending
            } else {
                TransportSettlementStatus::Settled
            }
        }

        fn next_poll_at(&self) -> Option<Instant> {
            self.ready_at
        }
    }

    impl TransportSettlementJob for RetryOnceQuarantineProjectionJob {
        fn settlement_status_until(&mut self, _deadline: Instant) -> TransportSettlementStatus {
            TransportSettlementStatus::Failed
        }

        fn project_quarantine(
            &mut self,
            _failure: TransportSettlementFailureKind,
        ) -> anyhow::Result<()> {
            let attempt = self.attempts.fetch_add(1, Ordering::AcqRel) + 1;
            if attempt == 1 {
                anyhow::bail!("injected first quarantine projection failure");
            }
            Ok(())
        }
    }

    impl TransportSettlementJob for ProjectingFailedSettlementJob {
        fn settlement_status_until(&mut self, _deadline: Instant) -> TransportSettlementStatus {
            TransportSettlementStatus::Failed
        }

        fn context(&self) -> TransportSettlementJobContext {
            TransportSettlementJobContext {
                job_kind: "test_failed_session",
                session_id: Some(self.session_id.clone()),
            }
        }

        fn project_quarantine(
            &mut self,
            _failure: TransportSettlementFailureKind,
        ) -> anyhow::Result<()> {
            self.projected.store(true, Ordering::Release);
            Ok(())
        }
    }

    #[test]
    fn settlement_cleanup_runtime_outlives_manager_while_job_pending() {
        let manager = RemoteDesktopTransportManager::new();
        let queue = manager.settlement_queue();
        let cleanup_runtime = queue.cleanup_runtime();
        queue.enqueue(PendingCleanupRuntimeOwner {
            _runtime: cleanup_runtime.clone(),
        });

        drop(manager);
        drop(queue);
        thread::sleep(TRANSPORT_SETTLEMENT_EXECUTOR_SLICE * 2);
        cleanup_runtime.block_on(async { tokio::task::yield_now().await });
    }

    #[test]
    fn settlement_executor_quarantines_panicking_job_without_dropping_owner() {
        let manager = RemoteDesktopTransportManager::new();
        let queue = manager.settlement_queue();
        let quarantine = Arc::downgrade(&queue.quarantine.records);
        queue.enqueue(PanickingSettlementJob);
        let deadline = Instant::now() + Duration::from_secs(1);
        while queue.quarantined_job_count() == 0 {
            assert!(
                Instant::now() < deadline,
                "panicking settlement job was neither retained nor quarantined"
            );
            thread::yield_now();
        }
        assert_eq!(queue.quarantined_job_count(), 1);
        drop(manager);
        drop(queue);
        thread::sleep(TRANSPORT_SETTLEMENT_EXECUTOR_SLICE * 2);
        assert!(
            quarantine.upgrade().is_some(),
            "executor shutdown cannot drop quarantined transport ownership"
        );
    }

    #[test]
    fn failed_settlement_closes_admission_and_projects_typed_health() {
        let manager = RemoteDesktopTransportManager::new();
        let projected = Arc::new(AtomicBool::new(false));
        manager
            .settlement_queue()
            .enqueue(ProjectingFailedSettlementJob {
                projected: Arc::clone(&projected),
                session_id: "rd-quarantine-health".to_string(),
            });
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let health = manager.settlement_health();
            if health.quarantined_jobs() == 1 {
                assert!(!health.admission_open());
                let view = health.to_value();
                assert_eq!(view["status"], json!("unhealthy"));
                assert_eq!(view["admission_open"], json!(false));
                assert_eq!(
                    view["failures"][0]["job_kind"],
                    json!("test_failed_session")
                );
                assert_eq!(
                    view["failures"][0]["session_id"],
                    json!("rd-quarantine-health")
                );
                if view["failures"][0]["projection_complete"] == json!(true) {
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "failed job was not quarantined and projected"
            );
            thread::yield_now();
        }
        assert!(
            projected.load(Ordering::Acquire),
            "typed quarantine must project an operational outcome"
        );
    }

    #[test]
    fn quarantine_projection_retries_after_queue_becomes_idle() {
        let manager = RemoteDesktopTransportManager::new();
        let attempts = Arc::new(AtomicUsize::new(0));
        manager
            .settlement_queue()
            .enqueue(RetryOnceQuarantineProjectionJob {
                attempts: Arc::clone(&attempts),
            });
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let health = manager.settlement_health().to_value();
            if health["quarantined_jobs"] == json!(1)
                && health["pending_outcome_projections"] == json!(0)
            {
                assert_eq!(health["failures"][0]["projection_attempts"], json!(2));
                assert_eq!(health["failures"][0]["last_projection_error"], Value::Null);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "idle settlement executor did not retry quarantine projection"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(attempts.load(Ordering::Acquire), 2);
    }

    #[test]
    fn delayed_pending_job_is_not_polled_before_its_ready_time() {
        let manager = RemoteDesktopTransportManager::new();
        let attempts = Arc::new(AtomicUsize::new(0));
        manager
            .settlement_queue()
            .enqueue(DelayedPendingSettlementJob {
                attempts: Arc::clone(&attempts),
                ready_at: None,
            });
        let first_deadline = Instant::now() + Duration::from_secs(1);
        while attempts.load(Ordering::Acquire) == 0 {
            assert!(Instant::now() < first_deadline, "job was not observed once");
            thread::yield_now();
        }
        thread::sleep(Duration::from_millis(60));
        assert_eq!(
            attempts.load(Ordering::Acquire),
            1,
            "executor polled a delayed job before next_poll_at"
        );
        let settled_deadline = Instant::now() + Duration::from_secs(1);
        while attempts.load(Ordering::Acquire) < 2 {
            assert!(
                Instant::now() < settled_deadline,
                "executor did not wake the delayed job at next_poll_at"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(attempts.load(Ordering::Acquire), 2);
    }

    #[test]
    fn quarantine_owner_latch_keeps_admission_closed_during_projection() {
        let manager = RemoteDesktopTransportManager::new();
        let (started_tx, started_rx) = channel();
        let (release_tx, release_rx) = channel();
        manager
            .settlement_queue()
            .enqueue(BlockingQuarantineProjectionJob {
                started: Some(started_tx),
                release: release_rx,
            });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("quarantine projection starts");

        let health = manager.settlement_health();
        assert_eq!(health.quarantined_jobs(), 1);
        assert!(
            !health.admission_open(),
            "moving a quarantine record through outcome projection must never reopen admission"
        );
        release_tx
            .send(())
            .expect("quarantine projection receives release");
    }

    #[test]
    fn admission_permit_linearizes_before_quarantine_projection() {
        let manager = Arc::new(RemoteDesktopTransportManager::new());
        let admission = manager
            .acquire_session_admission()
            .expect("healthy manager grants admission permit");
        let (started_tx, started_rx) = channel();
        let (release_tx, release_rx) = channel();
        manager
            .settlement_queue()
            .enqueue(BlockingQuarantineProjectionJob {
                started: Some(started_tx),
                release: release_rx,
            });
        let deadline = Instant::now() + Duration::from_secs(1);
        while manager.settlement_health().admission_open() {
            assert!(
                Instant::now() < deadline,
                "quarantine owner did not publish fail-closed admission"
            );
            thread::yield_now();
        }
        assert_eq!(
            started_rx.recv_timeout(Duration::from_millis(50)),
            Err(RecvTimeoutError::Timeout),
            "quarantine outcome projection crossed an in-flight admission permit"
        );

        let (rejected_tx, rejected_rx) = channel();
        let manager_for_rejection = Arc::clone(&manager);
        let rejection = thread::spawn(move || {
            let rejected = manager_for_rejection.acquire_session_admission().is_err();
            let _ = rejected_tx.send(rejected);
        });
        drop(admission);
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("projector crosses admission barrier after permit release");
        assert!(
            rejected_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("later admission completes"),
            "admission acquired after quarantine linearization"
        );
        rejection.join().expect("admission rejection thread joins");
        release_tx
            .send(())
            .expect("quarantine projection receives release");
    }

    #[test]
    fn executor_unavailable_never_projects_quarantine_on_submitter() {
        let queue = TransportSettlementQueue::with_disconnected_executor_for_test();
        let (started_tx, started_rx) = channel();
        let (release_tx, release_rx) = channel();
        let (returned_tx, returned_rx) = channel();
        let submitter = thread::spawn(move || {
            queue.enqueue(BlockingQuarantineProjectionJob {
                started: Some(started_tx),
                release: release_rx,
            });
            let _ = returned_tx.send(());
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("asynchronous quarantine projection starts");
        returned_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("submitter returns while quarantine projection is blocked");
        release_tx
            .send(())
            .expect("quarantine projection receives release");
        submitter.join().expect("submitter joins");
    }

    #[test]
    fn endpoint_cleanup_quarantine_emits_negative_completion_receipt() {
        let manager = Arc::new(RemoteDesktopTransportManager::new());
        let epoch = manager.allocate_epoch();
        let reservation = manager
            .reserve_endpoint("rd-cleanup-quarantine".to_string(), epoch)
            .expect("endpoint reservation succeeds");
        reservation.complete_with_cleanup_owner(|_| {
            panic!("injected endpoint cleanup panic");
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let health = manager.settlement_health();
            if health.quarantined_jobs() == 1 && health.pending_outcome_projections == 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "endpoint cleanup quarantine did not project a negative receipt"
            );
            thread::yield_now();
        }
        let mut retired = manager
            .take_endpoint_for_settlement("rd-cleanup-quarantine")
            .expect("manager retains pending reservation receiver");
        assert_eq!(
            retired.settlement_status_until(Instant::now() + Duration::from_secs(1)),
            TransportSettlementStatus::Failed,
            "negative cleanup receipt must fail parent transport settlement"
        );
    }

    #[test]
    fn allocator_is_monotonic_and_advances_past_recovered_epochs() {
        let manager = RemoteDesktopTransportManager::new();
        let first = manager.allocate_epoch();
        let second = manager.allocate_epoch();
        assert!(second > first);

        let recovered = second.value().saturating_add(10_000);
        manager.observe_prior_epoch(recovered);
        let resumed = manager.allocate_epoch();
        assert!(resumed.value() > recovered);
    }

    #[test]
    fn terminal_seal_cancels_and_settles_pending_endpoint_reservation() {
        let manager = Arc::new(RemoteDesktopTransportManager::new());
        let epoch = manager.allocate_epoch();
        let reservation = manager
            .reserve_endpoint("rd-pending-terminal".to_string(), epoch)
            .expect("live session reserves one endpoint generation");
        let mut stop_rx = reservation.stop_receiver();

        assert!(manager
            .reserve_endpoint("rd-pending-terminal".to_string(), epoch)
            .is_err());
        let retired = manager
            .take_endpoint_for_settlement("rd-pending-terminal")
            .expect("terminal sweep owns pending endpoint reservation");
        assert!(*stop_rx.borrow_and_update());

        reservation.complete_without_endpoint();
        assert!(retired.settle());
        assert!(manager
            .reserve_endpoint("rd-pending-terminal".to_string(), manager.allocate_epoch(),)
            .is_err());
    }

    #[test]
    fn dropped_pending_reservation_cannot_forge_settlement_receipt() {
        let manager = Arc::new(RemoteDesktopTransportManager::new());
        let reservation = manager
            .reserve_endpoint("rd-pending-unproven".to_string(), manager.allocate_epoch())
            .expect("endpoint setup reserves admission");
        let retired = manager
            .take_endpoint_for_settlement("rd-pending-unproven")
            .expect("terminal sweep owns pending endpoint reservation");

        drop(reservation);
        assert!(
            !retired.settle(),
            "dropping a setup token cannot prove partially-created resources closed"
        );
    }

    #[test]
    fn dropped_reservation_before_terminal_remains_manager_visible() {
        let manager = Arc::new(RemoteDesktopTransportManager::new());
        let reservation = manager
            .reserve_endpoint(
                "rd-drop-before-terminal".to_string(),
                manager.allocate_epoch(),
            )
            .expect("endpoint setup reserves admission");

        drop(reservation);
        let mut retired = manager
            .take_endpoint_for_settlement("rd-drop-before-terminal")
            .expect("terminal sweep must retain an unproven dropped setup reservation");
        assert_eq!(
            retired.settlement_status_until(Instant::now() + Duration::from_secs(1)),
            TransportSettlementStatus::Failed,
            "drop-first cannot make an unknown partially-created peer invisible"
        );
    }

    #[test]
    fn newer_endpoint_reservation_cancels_and_fences_older_generation() {
        let manager = Arc::new(RemoteDesktopTransportManager::new());
        let older_epoch = manager.allocate_epoch();
        let older = manager
            .reserve_endpoint("rd-pending-generation".to_string(), older_epoch)
            .expect("first endpoint generation reserves");
        let mut older_stop_rx = older.stop_receiver();
        let newer_epoch = manager.allocate_epoch();
        let newer = manager
            .reserve_endpoint("rd-pending-generation".to_string(), newer_epoch)
            .expect("newer endpoint generation advances admission");
        let mut newer_stop_rx = newer.stop_receiver();

        assert!(*older_stop_rx.borrow_and_update());
        assert!(!*newer_stop_rx.borrow_and_update());
        assert!(manager
            .reserve_endpoint("rd-pending-generation".to_string(), older_epoch)
            .is_err());
        assert!(manager
            .reserve_endpoint("rd-pending-generation".to_string(), newer_epoch)
            .is_err());

        let retired = manager
            .take_endpoint_for_settlement("rd-pending-generation")
            .expect("terminal sweep owns both pending generations");
        assert!(*newer_stop_rx.borrow_and_update());
        older.complete_without_endpoint();
        newer.complete_without_endpoint();
        assert!(retired.settle());
    }

    #[test]
    fn stale_preview_activation_cannot_replace_newer_generation() {
        let manager = RemoteDesktopTransportManager::new();
        let (current_stop_tx, mut current_stop_rx) = watch::channel(false);
        let (current_done_tx, current_done_rx) = channel();
        manager.activate_preview(
            "rd-preview-generation".to_string(),
            PreviewTransportEpoch::new(2),
            current_stop_tx,
            current_done_rx,
        );

        let (stale_stop_tx, mut stale_stop_rx) = watch::channel(false);
        let (stale_done_tx, stale_done_rx) = channel();
        manager.activate_preview(
            "rd-preview-generation".to_string(),
            PreviewTransportEpoch::new(1),
            stale_stop_tx,
            stale_done_rx,
        );

        assert!(*stale_stop_rx.borrow_and_update());
        assert!(!*current_stop_rx.borrow_and_update());
        stale_done_tx
            .send(PreviewTaskGroupCompletion)
            .expect("stale preview reaper retains completion ownership");

        let current = manager
            .take_preview_for_settlement("rd-preview-generation")
            .expect("newest preview remains registered");
        assert!(*current_stop_rx.borrow_and_update());
        current_done_tx
            .send(PreviewTaskGroupCompletion)
            .expect("current preview settlement retains completion ownership");
        assert!(current.settle());
    }

    #[test]
    fn retired_preview_settlement_waits_for_worker_group_completion() {
        let manager = RemoteDesktopTransportManager::new();
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let (worker_done_tx, worker_done_rx) = channel();
        manager.activate_preview(
            "rd-preview-settlement".to_string(),
            PreviewTransportEpoch::new(1),
            stop_tx,
            worker_done_rx,
        );
        let retired = manager
            .take_preview_for_settlement("rd-preview-settlement")
            .expect("preview is owned by manager");
        assert!(*stop_rx.borrow_and_update());

        let (settled_tx, settled_rx) = channel();
        let waiter = thread::spawn(move || {
            assert!(retired.settle());
            let _ = settled_tx.send(());
        });
        assert_eq!(
            settled_rx.recv_timeout(Duration::from_millis(25)),
            Err(RecvTimeoutError::Timeout),
            "session must not publish Closed before preview workers complete"
        );
        worker_done_tx
            .send(PreviewTaskGroupCompletion)
            .expect("worker group completion receipt sends");
        settled_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("settlement completes after worker receipt");
        waiter.join().expect("settlement waiter exits");
    }

    #[test]
    fn disconnected_preview_completion_cannot_prove_settlement() {
        let manager = RemoteDesktopTransportManager::new();
        let (stop_tx, _stop_rx) = watch::channel(false);
        let (worker_done_tx, worker_done_rx) = channel();
        manager.activate_preview(
            "rd-preview-disconnected".to_string(),
            PreviewTransportEpoch::new(1),
            stop_tx,
            worker_done_rx,
        );
        drop(worker_done_tx);

        let retired = manager
            .take_preview_for_settlement("rd-preview-disconnected")
            .expect("preview is owned by manager");
        assert!(
            !retired.settle(),
            "channel disconnect is not a worker-group completion receipt"
        );
    }

    #[test]
    fn direct_endpoint_settlement_is_bounded_when_worker_does_not_exit() {
        let (release_tx, release_rx) = channel();
        let (worker_exited_tx, worker_exited_rx) = channel();
        let completion = thread::spawn(move || {
            let _ = release_rx.recv();
            let _ = worker_exited_tx.send(());
        });
        let mut retired = RetiredDirectWebRtcEndpoint {
            completion: Some(completion),
            pending: VecDeque::new(),
            failed: false,
            settlement_queue: None,
        };
        let started = Instant::now();
        assert!(!retired.settle_until(started + Duration::from_millis(50)));
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "hung platform media worker must not own the caller shutdown deadline"
        );
        release_tx
            .send(())
            .expect("blocked endpoint worker releases");
        worker_exited_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("detached endpoint worker exits after release");
        assert_eq!(
            retired.settlement_status_until(Instant::now() + Duration::from_secs(1)),
            TransportSettlementStatus::Settled,
            "a bounded timeout must retain join ownership for a later settlement proof"
        );
    }

    #[test]
    fn pending_reservation_timeout_retains_completion_ownership() {
        let (completion_tx, completion_rx) = channel();
        let mut retired = RetiredDirectWebRtcEndpoint {
            completion: None,
            pending: VecDeque::from([completion_rx]),
            failed: false,
            settlement_queue: None,
        };

        assert_eq!(
            retired.settlement_status_until(Instant::now() + Duration::from_millis(20)),
            TransportSettlementStatus::Pending
        );
        completion_tx
            .send(DirectWebRtcReservationCompletion { settled: true })
            .expect("pending endpoint setup publishes its real cleanup receipt");
        assert_eq!(
            retired.settlement_status_until(Instant::now() + Duration::from_secs(1)),
            TransportSettlementStatus::Settled,
            "timeout cannot drain or discard the pending setup receipt receiver"
        );
    }

    #[test]
    fn endpoint_cleanup_remains_visible_to_concurrent_terminal_settlement() {
        let manager = Arc::new(RemoteDesktopTransportManager::new());
        let epoch = manager.allocate_epoch();
        let reservation = manager
            .reserve_endpoint("rd-cleanup-terminal-race".to_string(), epoch)
            .expect("endpoint setup reserves admission");
        let (cleanup_started_tx, cleanup_started_rx) = channel();
        let (release_cleanup_tx, release_cleanup_rx) = channel();
        let mut cleanup_started_tx = Some(cleanup_started_tx);
        reservation.complete_with_cleanup_owner(move |_| {
            if let Some(started) = cleanup_started_tx.take() {
                let _ = started.send(());
            }
            let _ = release_cleanup_rx.recv();
            true
        });
        cleanup_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("setup cleanup owns the partially-created endpoint");

        let mut retired = manager
            .take_endpoint_for_settlement("rd-cleanup-terminal-race")
            .expect("terminal sweep sees setup cleanup as pending transport ownership");
        assert_eq!(
            retired.settlement_status_until(Instant::now() + Duration::from_millis(20)),
            TransportSettlementStatus::Pending,
            "terminal settlement cannot publish Closed while setup cleanup still owns the peer"
        );
        release_cleanup_tx
            .send(())
            .expect("cleanup receives real peer-close completion");
        assert_eq!(
            retired.settlement_status_until(Instant::now() + Duration::from_secs(1)),
            TransportSettlementStatus::Settled
        );
    }

    #[test]
    fn negative_receipt_does_not_discard_remaining_transport_ownership() {
        let (failed_tx, failed_rx) = channel();
        failed_tx
            .send(DirectWebRtcReservationCompletion { settled: false })
            .expect("first setup reports explicit cleanup failure");
        let (pending_tx, pending_rx) = channel();
        let (release_worker_tx, release_worker_rx) = channel();
        let completion = thread::spawn(move || {
            let _ = release_worker_rx.recv();
        });
        let mut retired = RetiredDirectWebRtcEndpoint {
            completion: Some(completion),
            pending: VecDeque::from([failed_rx, pending_rx]),
            failed: false,
            settlement_queue: None,
        };

        assert_eq!(
            retired.settlement_status_until(Instant::now() + Duration::from_millis(20)),
            TransportSettlementStatus::Pending,
            "one negative receipt cannot discard a later reservation receiver or active worker handle"
        );
        assert_eq!(retired.pending.len(), 1);
        assert!(retired.completion.is_some());

        pending_tx
            .send(DirectWebRtcReservationCompletion { settled: true })
            .expect("remaining setup publishes real settlement");
        release_worker_tx
            .send(())
            .expect("active worker publishes completion");
        assert_eq!(
            retired.settlement_status_until(Instant::now() + Duration::from_secs(1)),
            TransportSettlementStatus::Failed,
            "explicit failure remains visible only after every other owner has settled"
        );
        assert!(retired.pending.is_empty());
        assert!(retired.completion.is_none());
    }
}
