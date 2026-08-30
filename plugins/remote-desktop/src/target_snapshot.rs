// EasyNet CLI — remote desktop host target snapshot execution
// ============================================================
//
// File: plugins/remote-desktop/src/target_snapshot.rs
// Description: Killable, deadline-bounded native target observation boundary.
//
// Protocol responsibility:
// - None. This module does not mint authority, mutate sessions, or carry media.
//
// Implementation approach:
// - Production sampling runs in the plugin-private
//   easynet-remoteapp-native-host executable. Independent inventory and guard
//   supervisors serialize requests through bounded mailboxes and a versioned
//   length-prefixed protocol.
// - A timed-out or invalid helper generation is killed and reaped before a new
//   native sample is admitted. Native APIs can therefore hang without making
//   the parent daemon permanently unavailable.
// - Unit tests may inject an in-process sampler. That path is compiled only for
//   tests and is not a production fallback.
//
// Architectural position:
// - Remote Desktop plugin execution isolation. Runtime invocation semantics,
//   target lifecycle commits, input policy, and WebRTC remain in their owning
//   parent components.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::mpsc::Receiver;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(test)]
use easynet_remoteapp_native_protocol::{
    read_frame as read_helper_frame, write_frame as write_helper_frame,
    FrameError as HelperFrameError, TargetObservationSample as NativeTargetObservationSample,
    MAX_FRAME_BYTES as HELPER_MAX_FRAME_BYTES, PROTOCOL as HELPER_PROTOCOL,
    REQUEST_KIND as HELPER_REQUEST_KIND, RESPONSE_KIND as HELPER_RESPONSE_KIND,
    SCHEMA_VERSION as HELPER_SCHEMA_VERSION,
};
use easynet_remoteapp_native_protocol::{Request as HelperRequest, Response as HelperResponse};

#[cfg(test)]
use crate::daemon::plugins::remote_desktop::native_host_process::sibling_executable;
use crate::daemon::plugins::remote_desktop::native_host_process::NativeHostProcess;
#[cfg(test)]
use crate::daemon::plugins::remote_desktop::session::now_ms;
use crate::daemon::plugins::remote_desktop::target_observer::PlatformTargetObservationSample;
const SNAPSHOT_MAILBOX_CAPACITY: usize = 8;
const HELPER_WAIT_SLICE: Duration = Duration::from_millis(10);
const HELPER_RESTART_BACKOFF_BASE: Duration = Duration::from_millis(50);
const HELPER_RESTART_BACKOFF_MAX: Duration = Duration::from_secs(2);
#[cfg(test)]
const HELPER_TEST_FAULT_ENV: &str = "EASYNET_REMOTEAPP_NATIVE_HOST_TEST_FAULT";

#[cfg(test)]
pub(in crate::daemon::plugins::remote_desktop) trait TargetObservationSampler:
    Send + Sync
{
    fn sample(&self) -> PlatformTargetObservationSample;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum TargetSnapshotOwner {
    MonitorGeneration(u64),
    InputRequest(u64),
}

impl TargetSnapshotOwner {
    const fn is_input(self) -> bool {
        matches!(self, Self::InputRequest(_))
    }
}

#[derive(Debug)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetSnapshotSample {
    observation: PlatformTargetObservationSample,
    started_at_ms: u64,
    completed_at_ms: u64,
}

impl TargetSnapshotSample {
    pub(in crate::daemon::plugins::remote_desktop) fn observation(
        &self,
    ) -> &PlatformTargetObservationSample {
        &self.observation
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn completed_at_ms(&self) -> u64 {
        self.completed_at_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum TargetSnapshotDeadlineError {
    DeadlineExceeded {
        request_id: u64,
        owner: TargetSnapshotOwner,
    },
    QueueFull {
        request_id: u64,
        owner: TargetSnapshotOwner,
    },
    SequenceExhausted(&'static str),
    SpawnFailed(String),
    ProcessUnavailable {
        request_id: u64,
        owner: TargetSnapshotOwner,
    },
    ProtocolFailed {
        request_id: u64,
        owner: TargetSnapshotOwner,
    },
    WorkerFailed {
        request_id: u64,
        owner: TargetSnapshotOwner,
    },
}

impl std::fmt::Display for TargetSnapshotDeadlineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeadlineExceeded { request_id, owner } => write!(
                formatter,
                "host target snapshot request {request_id} owned by {owner:?} exceeded deadline"
            ),
            Self::QueueFull { request_id, owner } => write!(
                formatter,
                "host target snapshot request {request_id} owned by {owner:?} exceeded the bounded mailbox"
            ),
            Self::SequenceExhausted(counter) => {
                write!(formatter, "host target snapshot {counter} sequence exhausted")
            }
            Self::SpawnFailed(detail) => {
                write!(formatter, "host target snapshot helper spawn failed: {detail}")
            }
            Self::ProcessUnavailable { request_id, owner } => write!(
                formatter,
                "host target snapshot helper circuit is open for request {request_id} owned by {owner:?}"
            ),
            Self::ProtocolFailed { request_id, owner } => write!(
                formatter,
                "host target snapshot helper protocol failed for request {request_id} owned by {owner:?}"
            ),
            Self::WorkerFailed { request_id, owner } => write!(
                formatter,
                "host target snapshot request {request_id} owned by {owner:?} failed"
            ),
        }
    }
}

pub(in crate::daemon::plugins::remote_desktop) struct TargetSnapshotDeadlineExecutor {
    request_sequence: AtomicU64,
    input_sequence: AtomicU64,
    backend: TargetSnapshotBackend,
}

enum TargetSnapshotBackend {
    Process(ProcessTargetSnapshotExecutor),
    #[cfg(test)]
    Injected(InProcessTargetSnapshotExecutor),
}

impl std::fmt::Debug for TargetSnapshotDeadlineExecutor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TargetSnapshotDeadlineExecutor")
            .field(
                "request_sequence",
                &self.request_sequence.load(Ordering::Acquire),
            )
            .field(
                "input_sequence",
                &self.input_sequence.load(Ordering::Acquire),
            )
            .field(
                "backend",
                &match &self.backend {
                    TargetSnapshotBackend::Process(_) => "killable_process",
                    #[cfg(test)]
                    TargetSnapshotBackend::Injected(_) => "test_injected_sampler",
                },
            )
            .finish()
    }
}

impl TargetSnapshotDeadlineExecutor {
    pub(in crate::daemon::plugins::remote_desktop) fn platform() -> Self {
        Self {
            request_sequence: AtomicU64::new(0),
            input_sequence: AtomicU64::new(0),
            backend: TargetSnapshotBackend::Process(ProcessTargetSnapshotExecutor::new()),
        }
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        sampler: Arc<dyn TargetObservationSampler>,
    ) -> Self {
        Self {
            request_sequence: AtomicU64::new(0),
            input_sequence: AtomicU64::new(0),
            backend: TargetSnapshotBackend::Injected(InProcessTargetSnapshotExecutor::new(sampler)),
        }
    }

    #[cfg(all(test, feature = "remoteapp-e2e-fault-injection"))]
    fn platform_with_fault_for_test(fault: String) -> Self {
        Self {
            request_sequence: AtomicU64::new(0),
            input_sequence: AtomicU64::new(0),
            backend: TargetSnapshotBackend::Process(ProcessTargetSnapshotExecutor::with_launch(
                NativeObservationHelperLaunch {
                    test_fault: Some(fault),
                },
            )),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn sample_for_generation(
        &self,
        generation: u64,
        timeout: Duration,
    ) -> Result<PlatformTargetObservationSample, TargetSnapshotDeadlineError> {
        self.sample_for_owner(TargetSnapshotOwner::MonitorGeneration(generation), timeout)
            .map(|sample| sample.observation)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn sample_for_input(
        &self,
        timeout: Duration,
    ) -> Result<TargetSnapshotSample, TargetSnapshotDeadlineError> {
        let request = next_sequence(&self.input_sequence)
            .ok_or(TargetSnapshotDeadlineError::SequenceExhausted("input"))?;
        self.sample_for_owner(TargetSnapshotOwner::InputRequest(request), timeout)
    }

    fn sample_for_owner(
        &self,
        owner: TargetSnapshotOwner,
        timeout: Duration,
    ) -> Result<TargetSnapshotSample, TargetSnapshotDeadlineError> {
        let request_id = next_sequence(&self.request_sequence)
            .ok_or(TargetSnapshotDeadlineError::SequenceExhausted("request"))?;
        match &self.backend {
            TargetSnapshotBackend::Process(executor) => executor.sample(request_id, owner, timeout),
            #[cfg(test)]
            TargetSnapshotBackend::Injected(executor) => executor.sample(owner, timeout),
        }
    }
}

fn next_sequence(sequence: &AtomicU64) -> Option<u64> {
    sequence
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1)
        })
        .ok()
        .and_then(|previous| previous.checked_add(1))
}

struct SnapshotRequest {
    request_id: u64,
    owner: TargetSnapshotOwner,
    deadline: Instant,
    response: SyncSender<Result<TargetSnapshotSample, TargetSnapshotDeadlineError>>,
}

impl SnapshotRequest {
    fn deadline_error(&self) -> TargetSnapshotDeadlineError {
        TargetSnapshotDeadlineError::DeadlineExceeded {
            request_id: self.request_id,
            owner: self.owner,
        }
    }

    fn queue_full_error(&self) -> TargetSnapshotDeadlineError {
        TargetSnapshotDeadlineError::QueueFull {
            request_id: self.request_id,
            owner: self.owner,
        }
    }
}

#[derive(Default)]
struct SnapshotMailboxState {
    input: VecDeque<SnapshotRequest>,
    monitor: VecDeque<SnapshotRequest>,
    shutdown: bool,
}

struct SnapshotMailbox {
    state: Mutex<SnapshotMailboxState>,
    ready: Condvar,
}

impl SnapshotMailbox {
    fn new() -> Self {
        Self {
            state: Mutex::new(SnapshotMailboxState::default()),
            ready: Condvar::new(),
        }
    }

    fn enqueue(&self, request: SnapshotRequest) -> Result<(), TargetSnapshotDeadlineError> {
        let mut state = self.lock();
        if state.shutdown {
            return Err(TargetSnapshotDeadlineError::WorkerFailed {
                request_id: request.request_id,
                owner: request.owner,
            });
        }
        let queued = state.input.len() + state.monitor.len();
        if queued >= SNAPSHOT_MAILBOX_CAPACITY {
            if request.owner.is_input() {
                if let Some(evicted) = state.monitor.pop_back() {
                    let _ = evicted.response.send(Err(evicted.queue_full_error()));
                } else {
                    return Err(request.queue_full_error());
                }
            } else {
                return Err(request.queue_full_error());
            }
        }
        if request.owner.is_input() {
            state.input.push_back(request);
        } else {
            state.monitor.push_back(request);
        }
        self.ready.notify_one();
        Ok(())
    }

    fn next(&self) -> Option<SnapshotRequest> {
        let mut state = self.lock();
        loop {
            if state.shutdown {
                return None;
            }
            if let Some(request) = state.input.pop_front() {
                return Some(request);
            }
            if let Some(request) = state.monitor.pop_front() {
                return Some(request);
            }
            state = match self.ready.wait(state) {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
    }

    fn is_shutdown(&self) -> bool {
        self.lock().shutdown
    }

    fn shutdown(&self) {
        let drained = {
            let mut state = self.lock();
            state.shutdown = true;
            let mut drained = state.input.drain(..).collect::<Vec<_>>();
            drained.extend(state.monitor.drain(..));
            drained
        };
        for request in drained {
            let _ = request
                .response
                .send(Err(TargetSnapshotDeadlineError::WorkerFailed {
                    request_id: request.request_id,
                    owner: request.owner,
                }));
        }
        self.ready.notify_all();
    }

    fn lock(&self) -> MutexGuard<'_, SnapshotMailboxState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

struct ProcessTargetSnapshotExecutor {
    inventory: ProcessTargetSnapshotLane,
    guard: ProcessTargetSnapshotLane,
}

impl ProcessTargetSnapshotExecutor {
    fn new() -> Self {
        Self::with_launch(NativeObservationHelperLaunch::default())
    }

    fn with_launch(launch: NativeObservationHelperLaunch) -> Self {
        Self {
            inventory: ProcessTargetSnapshotLane::new("inventory", launch.clone()),
            guard: ProcessTargetSnapshotLane::new("guard", launch),
        }
    }

    fn sample(
        &self,
        request_id: u64,
        owner: TargetSnapshotOwner,
        timeout: Duration,
    ) -> Result<TargetSnapshotSample, TargetSnapshotDeadlineError> {
        if owner.is_input() {
            self.guard.sample(request_id, owner, timeout)
        } else {
            self.inventory.sample(request_id, owner, timeout)
        }
    }
}

struct ProcessTargetSnapshotLane {
    mailbox: Arc<SnapshotMailbox>,
    supervisor: Mutex<Option<JoinHandle<()>>>,
    startup_error: Option<String>,
}

impl ProcessTargetSnapshotLane {
    fn new(lane: &'static str, launch: NativeObservationHelperLaunch) -> Self {
        let mailbox = Arc::new(SnapshotMailbox::new());
        let worker_mailbox = Arc::clone(&mailbox);
        let spawn = thread::Builder::new()
            .name(format!("easynet-rd-{lane}-helper-supervisor"))
            .spawn(move || run_process_supervisor(worker_mailbox, launch));
        match spawn {
            Ok(supervisor) => Self {
                mailbox,
                supervisor: Mutex::new(Some(supervisor)),
                startup_error: None,
            },
            Err(error) => Self {
                mailbox,
                supervisor: Mutex::new(None),
                startup_error: Some(error.to_string()),
            },
        }
    }

    fn sample(
        &self,
        request_id: u64,
        owner: TargetSnapshotOwner,
        timeout: Duration,
    ) -> Result<TargetSnapshotSample, TargetSnapshotDeadlineError> {
        if let Some(error) = &self.startup_error {
            return Err(TargetSnapshotDeadlineError::SpawnFailed(error.clone()));
        }
        let deadline = Instant::now() + timeout;
        let (response, response_rx) = mpsc::sync_channel(1);
        self.mailbox.enqueue(SnapshotRequest {
            request_id,
            owner,
            deadline,
            response,
        })?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(TargetSnapshotDeadlineError::DeadlineExceeded { request_id, owner });
        }
        match response_rx.recv_timeout(remaining) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                Err(TargetSnapshotDeadlineError::DeadlineExceeded { request_id, owner })
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(TargetSnapshotDeadlineError::WorkerFailed { request_id, owner })
            }
        }
    }
}

impl Drop for ProcessTargetSnapshotLane {
    fn drop(&mut self) {
        self.mailbox.shutdown();
        let supervisor = match self.supervisor.get_mut() {
            Ok(supervisor) => supervisor,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(supervisor) = supervisor.take() {
            let _ = supervisor.join();
        }
    }
}

fn run_process_supervisor(mailbox: Arc<SnapshotMailbox>, launch: NativeObservationHelperLaunch) {
    let mut worker = NativeObservationHelperWorker::new(launch);
    while let Some(request) = mailbox.next() {
        let result = if Instant::now() >= request.deadline {
            Err(request.deadline_error())
        } else {
            worker.execute(&request, &mailbox)
        };
        let _ = request.response.send(result);
    }
    worker.shutdown();
}

struct NativeObservationHelperWorker {
    generation: Option<NativeHostProcess<HelperResponse>>,
    generation_sequence: u64,
    consecutive_failures: u32,
    retry_not_before: Option<Instant>,
    launch: NativeObservationHelperLaunch,
}

impl NativeObservationHelperWorker {
    fn new(launch: NativeObservationHelperLaunch) -> Self {
        Self {
            generation: None,
            generation_sequence: 0,
            consecutive_failures: 0,
            retry_not_before: None,
            launch,
        }
    }

    fn execute(
        &mut self,
        request: &SnapshotRequest,
        mailbox: &SnapshotMailbox,
    ) -> Result<TargetSnapshotSample, TargetSnapshotDeadlineError> {
        if self
            .retry_not_before
            .is_some_and(|retry| retry > Instant::now())
        {
            return Err(TargetSnapshotDeadlineError::ProcessUnavailable {
                request_id: request.request_id,
                owner: request.owner,
            });
        }
        if self.generation.is_none() {
            self.generation_sequence = self.generation_sequence.checked_add(1).ok_or(
                TargetSnapshotDeadlineError::SequenceExhausted("helper generation"),
            )?;
            match NativeHostProcess::spawn(
                self.generation_sequence,
                super::NATIVE_HOST_EXECUTABLE,
                "native-observation-helper",
                &self.launch.extra_environment(),
            ) {
                Ok(generation) => self.generation = Some(generation),
                Err(error) => {
                    self.record_failure();
                    return Err(TargetSnapshotDeadlineError::SpawnFailed(error.to_string()));
                }
            }
        }

        let generation_id = self
            .generation
            .as_ref()
            .expect("native observation helper generation exists after spawn")
            .id();
        if self
            .generation
            .as_ref()
            .expect("native observation helper generation exists before protocol check")
            .protocol_violated()
        {
            self.retire_failed_generation();
            return Err(TargetSnapshotDeadlineError::ProtocolFailed {
                request_id: request.request_id,
                owner: request.owner,
            });
        }
        let envelope = HelperRequest::sample_target_inventory(generation_id, request.request_id);
        if self
            .generation
            .as_mut()
            .expect("native observation helper generation exists before write")
            .write_request(&envelope)
            .is_err()
        {
            self.retire_failed_generation();
            return Err(TargetSnapshotDeadlineError::ProtocolFailed {
                request_id: request.request_id,
                owner: request.owner,
            });
        }

        loop {
            if mailbox.is_shutdown() {
                self.retire_failed_generation();
                return Err(TargetSnapshotDeadlineError::WorkerFailed {
                    request_id: request.request_id,
                    owner: request.owner,
                });
            }
            let remaining = request.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.retire_failed_generation();
                return Err(request.deadline_error());
            }
            let response = self
                .generation
                .as_ref()
                .expect("native observation helper generation exists while awaiting response")
                .recv_timeout(remaining.min(HELPER_WAIT_SLICE));
            match response {
                Ok(Ok(response)) => {
                    let protocol_violated = self
                        .generation
                        .as_ref()
                        .expect("native observation helper generation exists after response")
                        .protocol_violated();
                    if protocol_violated
                        || !response.matches_request(generation_id, request.request_id)
                    {
                        self.retire_failed_generation();
                        return Err(TargetSnapshotDeadlineError::ProtocolFailed {
                            request_id: request.request_id,
                            owner: request.owner,
                        });
                    }
                    let observation = match PlatformTargetObservationSample::from_native_host(
                        response.observation,
                    ) {
                        Ok(observation) => observation,
                        Err(_) => {
                            self.retire_failed_generation();
                            return Err(TargetSnapshotDeadlineError::ProtocolFailed {
                                request_id: request.request_id,
                                owner: request.owner,
                            });
                        }
                    };
                    self.record_success();
                    return Ok(TargetSnapshotSample {
                        observation,
                        started_at_ms: response.started_at_ms,
                        completed_at_ms: response.completed_at_ms,
                    });
                }
                Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => {
                    self.retire_failed_generation();
                    return Err(TargetSnapshotDeadlineError::ProtocolFailed {
                        request_id: request.request_id,
                        owner: request.owner,
                    });
                }
                Err(RecvTimeoutError::Timeout) => continue,
            }
        }
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.retry_not_before = None;
    }

    fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.retry_not_before =
            Some(Instant::now() + helper_restart_backoff(self.consecutive_failures));
    }

    fn retire_failed_generation(&mut self) {
        if let Some(mut generation) = self.generation.take() {
            generation.terminate();
        }
        self.record_failure();
    }

    fn shutdown(&mut self) {
        if let Some(mut generation) = self.generation.take() {
            generation.terminate();
        }
    }
}

fn helper_restart_backoff(failure_count: u32) -> Duration {
    let exponent = failure_count.saturating_sub(1).min(6);
    HELPER_RESTART_BACKOFF_BASE
        .saturating_mul(1_u32 << exponent)
        .min(HELPER_RESTART_BACKOFF_MAX)
}

#[derive(Clone, Default)]
struct NativeObservationHelperLaunch {
    #[cfg(test)]
    test_fault: Option<String>,
}

impl NativeObservationHelperLaunch {
    fn extra_environment(&self) -> Vec<(OsString, OsString)> {
        let environment = Vec::new();
        #[cfg(test)]
        let mut environment = environment;
        #[cfg(test)]
        if let Some(fault) = &self.test_fault {
            environment.push((HELPER_TEST_FAULT_ENV.into(), fault.into()));
        }
        environment
    }
}

#[cfg(test)]
struct InProcessTargetSnapshotExecutor {
    sampler: Arc<dyn TargetObservationSampler>,
    in_flight: Mutex<Option<InFlightTargetSnapshot>>,
    request_sequence: AtomicU64,
}

#[cfg(test)]
struct InFlightTargetSnapshot {
    request_id: u64,
    owner: TargetSnapshotOwner,
    result_rx: Receiver<TargetSnapshotSample>,
    _join: JoinHandle<()>,
}

#[cfg(test)]
impl InProcessTargetSnapshotExecutor {
    fn new(sampler: Arc<dyn TargetObservationSampler>) -> Self {
        Self {
            sampler,
            in_flight: Mutex::new(None),
            request_sequence: AtomicU64::new(0),
        }
    }

    fn sample(
        &self,
        owner: TargetSnapshotOwner,
        timeout: Duration,
    ) -> Result<TargetSnapshotSample, TargetSnapshotDeadlineError> {
        let deadline = Instant::now() + timeout;
        loop {
            let mut in_flight = match self.in_flight.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            if in_flight.is_none() {
                *in_flight = Some(self.spawn_request(owner)?);
            }
            let request = in_flight
                .as_mut()
                .expect("test target snapshot request exists after spawn");
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(TargetSnapshotDeadlineError::DeadlineExceeded {
                    request_id: request.request_id,
                    owner: request.owner,
                });
            }
            match request.result_rx.recv_timeout(remaining) {
                Ok(sample) => {
                    let completed = in_flight
                        .take()
                        .expect("completed test target snapshot request exists");
                    if completed.owner != owner {
                        continue;
                    }
                    return Ok(sample);
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(TargetSnapshotDeadlineError::DeadlineExceeded {
                        request_id: request.request_id,
                        owner: request.owner,
                    });
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let failed = in_flight
                        .take()
                        .expect("failed test target snapshot request exists");
                    if failed.owner != owner {
                        continue;
                    }
                    return Err(TargetSnapshotDeadlineError::WorkerFailed {
                        request_id: failed.request_id,
                        owner: failed.owner,
                    });
                }
            }
        }
    }

    fn spawn_request(
        &self,
        owner: TargetSnapshotOwner,
    ) -> Result<InFlightTargetSnapshot, TargetSnapshotDeadlineError> {
        let request_id = next_sequence(&self.request_sequence).ok_or(
            TargetSnapshotDeadlineError::SequenceExhausted("test request"),
        )?;
        let sampler = Arc::clone(&self.sampler);
        let (result_tx, result_rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name(format!("easynet-rd-test-target-snapshot-{request_id}"))
            .spawn(move || {
                let started_at_ms = now_ms();
                let observation = sampler.sample();
                let sample = TargetSnapshotSample {
                    observation,
                    started_at_ms,
                    completed_at_ms: now_ms(),
                };
                let _ = result_tx.send(sample);
            })
            .map_err(|error| TargetSnapshotDeadlineError::SpawnFailed(error.to_string()))?;
        Ok(InFlightTargetSnapshot {
            request_id,
            owner,
            result_rx,
            _join: join,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn helper_frames_round_trip_exact_versioned_envelope() {
        let request = HelperRequest {
            schema_version: HELPER_SCHEMA_VERSION,
            protocol: HELPER_PROTOCOL.to_string(),
            kind: HELPER_REQUEST_KIND.to_string(),
            process_generation: 9,
            request_id: 17,
        };
        let mut bytes = Vec::new();
        write_helper_frame(&mut bytes, &request).expect("encode helper request");
        let decoded: HelperRequest = read_helper_frame(&mut Cursor::new(bytes))
            .expect("decode helper request")
            .expect("request frame exists");
        assert_eq!(decoded.process_generation, 9);
        assert_eq!(decoded.request_id, 17);
        assert_eq!(decoded.schema_version, HELPER_SCHEMA_VERSION);
        assert_eq!(decoded.protocol, HELPER_PROTOCOL);
        assert_eq!(decoded.kind, HELPER_REQUEST_KIND);
    }

    #[test]
    fn helper_frame_rejects_oversized_length_before_allocation() {
        let mut bytes = ((HELPER_MAX_FRAME_BYTES + 1) as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(b"{}");
        assert!(matches!(
            read_helper_frame::<HelperRequest>(&mut Cursor::new(bytes)),
            Err(HelperFrameError::Oversized)
        ));
    }

    #[test]
    fn helper_frame_rejects_malformed_json() {
        let body = b"{not-json";
        let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(body);
        assert!(matches!(
            read_helper_frame::<HelperRequest>(&mut Cursor::new(bytes)),
            Err(HelperFrameError::Decode(_))
        ));
    }

    #[test]
    fn helper_sequence_exhaustion_fails_closed_without_reuse() {
        let sequence = AtomicU64::new(u64::MAX);
        assert_eq!(next_sequence(&sequence), None);
        assert_eq!(sequence.load(Ordering::Acquire), u64::MAX);
    }

    #[test]
    fn input_admission_evicts_monitor_work_but_never_another_input() {
        let mailbox = SnapshotMailbox::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut monitor_receivers = Vec::new();
        for request_id in 1..=SNAPSHOT_MAILBOX_CAPACITY as u64 {
            let (response, response_rx) = mpsc::sync_channel(1);
            mailbox
                .enqueue(SnapshotRequest {
                    request_id,
                    owner: TargetSnapshotOwner::MonitorGeneration(request_id),
                    deadline,
                    response,
                })
                .expect("monitor request enters bounded mailbox");
            monitor_receivers.push(response_rx);
        }
        let (input_response, _input_rx) = mpsc::sync_channel(1);
        mailbox
            .enqueue(SnapshotRequest {
                request_id: 99,
                owner: TargetSnapshotOwner::InputRequest(1),
                deadline,
                response: input_response,
            })
            .expect("input request evicts lower-priority monitor work");
        assert!(matches!(
            monitor_receivers
                .last()
                .expect("last monitor receiver")
                .try_recv(),
            Ok(Err(TargetSnapshotDeadlineError::QueueFull {
                request_id: 8,
                ..
            }))
        ));
        assert!(matches!(
            mailbox.next().map(|request| request.owner),
            Some(TargetSnapshotOwner::InputRequest(1))
        ));
    }

    #[test]
    fn helper_restart_backoff_is_exponential_and_bounded() {
        assert_eq!(helper_restart_backoff(1), Duration::from_millis(50));
        assert_eq!(helper_restart_backoff(2), Duration::from_millis(100));
        assert_eq!(helper_restart_backoff(7), Duration::from_secs(2));
        assert_eq!(helper_restart_backoff(u32::MAX), Duration::from_secs(2));
    }

    #[test]
    fn helper_response_rejects_stale_generation_request_and_time_regression() {
        let response = HelperResponse {
            schema_version: HELPER_SCHEMA_VERSION,
            protocol: HELPER_PROTOCOL.to_string(),
            kind: HELPER_RESPONSE_KIND.to_string(),
            process_generation: 7,
            request_id: 11,
            started_at_ms: 100,
            completed_at_ms: 101,
            observation: NativeTargetObservationSample::snapshot_failed("test observation", 100),
        };
        assert!(response.matches_request(7, 11));
        assert!(!response.matches_request(6, 11));
        assert!(!response.matches_request(7, 10));

        let regressed = HelperResponse {
            completed_at_ms: 99,
            ..response
        };
        assert!(!regressed.matches_request(7, 11));
    }

    #[test]
    fn process_executor_round_trips_real_sibling_helper_and_reaps_on_drop() {
        sibling_executable(super::super::NATIVE_HOST_EXECUTABLE).expect(
            "build the sibling native host before this process-boundary test with \
             `cargo build -p easynet-remoteapp-native-host`",
        );
        let executor = TargetSnapshotDeadlineExecutor::platform();
        let sample = executor
            .sample_for_input(Duration::from_secs(3))
            .expect("real native-host process returns one bounded sample");
        let _observation = sample.observation();
        let shutdown_started = Instant::now();
        drop(executor);
        assert!(
            shutdown_started.elapsed() < Duration::from_millis(500),
            "native-host supervisor shutdown must kill, reap, and join promptly"
        );
    }

    #[cfg(feature = "remoteapp-e2e-fault-injection")]
    #[test]
    fn hung_native_generation_is_killed_reaped_and_replaced() {
        sibling_executable(super::super::NATIVE_HOST_EXECUTABLE).expect(
            "build the sibling native host with \
             `cargo build -p easynet-remoteapp-native-host --features remoteapp-e2e-fault-injection`",
        );
        let temp = tempfile::tempdir().expect("fault marker tempdir");
        let marker = temp.path().join("first-generation-hung");
        let executor = TargetSnapshotDeadlineExecutor::platform_with_fault_for_test(format!(
            "hang_once:{}",
            marker.display()
        ));
        let hung_started = Instant::now();
        assert!(matches!(
            executor.sample_for_input(Duration::from_secs(1)),
            Err(TargetSnapshotDeadlineError::DeadlineExceeded { .. })
        ));
        assert!(hung_started.elapsed() < Duration::from_millis(1_500));
        assert!(
            marker.is_file(),
            "first helper generation reached the injected native hang"
        );
        thread::sleep(Duration::from_millis(100));
        executor
            .sample_for_input(Duration::from_secs(3))
            .expect("replacement helper generation serves a fresh sample");
        let shutdown_started = Instant::now();
        drop(executor);
        assert!(shutdown_started.elapsed() < Duration::from_millis(500));
    }

    #[cfg(feature = "remoteapp-e2e-fault-injection")]
    #[test]
    fn repeated_native_hangs_are_killed_and_reaped_before_restart() {
        sibling_executable(super::super::NATIVE_HOST_EXECUTABLE).expect(
            "build the sibling native host with \
             `cargo build -p easynet-remoteapp-native-host --features remoteapp-e2e-fault-injection`",
        );
        let executor =
            TargetSnapshotDeadlineExecutor::platform_with_fault_for_test("hang_always".to_string());
        for (attempt, backoff) in [50_u64, 100, 200].into_iter().enumerate() {
            assert!(matches!(
                executor.sample_for_input(Duration::from_millis(300)),
                Err(TargetSnapshotDeadlineError::DeadlineExceeded { .. })
            ));
            if attempt < 2 {
                thread::sleep(Duration::from_millis(backoff + 20));
            }
        }
        let shutdown_started = Instant::now();
        drop(executor);
        assert!(
            shutdown_started.elapsed() < Duration::from_millis(500),
            "all failed helper generations must already be reaped"
        );
    }
}
