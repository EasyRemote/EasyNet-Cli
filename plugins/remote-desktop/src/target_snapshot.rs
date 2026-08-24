// EasyNet CLI — remote desktop host target snapshot execution
// ============================================================
//
// One plugin-owned failure domain serializes every native window/display
// snapshot used by lifecycle observation and target-local input validation.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::daemon::plugins::remote_desktop::session::now_ms;
use crate::daemon::plugins::remote_desktop::target_observer::{
    sample_platform_target_observations, PlatformTargetObservationSample,
};

pub(in crate::daemon::plugins::remote_desktop) trait TargetObservationSampler:
    Send + Sync
{
    fn sample(&self) -> PlatformTargetObservationSample;
}

struct PlatformTargetObservationSampler;

impl TargetObservationSampler for PlatformTargetObservationSampler {
    fn sample(&self) -> PlatformTargetObservationSample {
        sample_platform_target_observations()
    }
}

pub(in crate::daemon::plugins::remote_desktop) struct TargetSnapshotDeadlineExecutor {
    sampler: Arc<dyn TargetObservationSampler>,
    in_flight: Mutex<Option<InFlightTargetSnapshot>>,
    request_sequence: AtomicU64,
    input_sequence: AtomicU64,
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
            .finish_non_exhaustive()
    }
}

struct InFlightTargetSnapshot {
    request_id: u64,
    owner: TargetSnapshotOwner,
    result_rx: Receiver<TargetSnapshotSample>,
    _join: JoinHandle<()>,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum TargetSnapshotOwner {
    MonitorGeneration(u64),
    InputRequest(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum TargetSnapshotDeadlineError {
    DeadlineExceeded {
        request_id: u64,
        owner: TargetSnapshotOwner,
    },
    SpawnFailed(String),
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
            Self::SpawnFailed(detail) => {
                write!(
                    formatter,
                    "host target snapshot worker spawn failed: {detail}"
                )
            }
            Self::WorkerFailed { request_id, owner } => write!(
                formatter,
                "host target snapshot request {request_id} owned by {owner:?} failed"
            ),
        }
    }
}

impl TargetSnapshotDeadlineExecutor {
    pub(in crate::daemon::plugins::remote_desktop) fn platform() -> Self {
        Self::new(Arc::new(PlatformTargetObservationSampler))
    }

    pub(in crate::daemon::plugins::remote_desktop) fn new(
        sampler: Arc<dyn TargetObservationSampler>,
    ) -> Self {
        Self {
            sampler,
            in_flight: Mutex::new(None),
            request_sequence: AtomicU64::new(0),
            input_sequence: AtomicU64::new(0),
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
        let request = self.input_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        self.sample_for_owner(TargetSnapshotOwner::InputRequest(request), timeout)
    }

    fn sample_for_owner(
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
                .expect("target snapshot request exists after spawn");
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
                        .expect("completed target snapshot request exists");
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
                        .expect("failed target snapshot request exists");
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
        let request_id = self.request_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let sampler = Arc::clone(&self.sampler);
        let (result_tx, result_rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name(format!("easynet-rd-target-snapshot-{request_id}"))
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
