// EasyNet CLI — remote desktop target monitor
// ===========================================
//
// File: plugins/remote-desktop/src/target_monitor.rs
// Description: Plugin-owned target lifecycle poller for remote desktop sessions.
//
// Protocol Responsibility:
// - None. This is daemon-owned device runtime lifecycle infrastructure.
//
// Implementation Approach:
// - Maintain one plugin-owned supervisor and one replaceable poll generation.
// - Track session ids after TARGET_BOUND/create_session and cancel them at
//   terminal cleanup.
// - Each worker tick samples host target state once, fans the immutable sample
//   out to tracked sessions, and commits observations only through
//   RemoteDesktopSessionStore, so session aggregate state remains the single
//   mutation boundary.
// - Rebuild a failed poll generation from supervisor-owned desired session
//   state. Repeated failures exhaust a bounded budget and explicitly mark
//   targets unavailable instead of silently leaving input enabled.
//
// Architectural Position:
// - Remote-desktop plugin lifecycle infrastructure, deliberately independent of
//   WebRTC/native media loops.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::daemon::plugins::remote_desktop::lifecycle_worker::LifecycleWorker;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::{now_ms, TargetMediaSourceLost};
use crate::daemon::plugins::remote_desktop::session_recovery::RemoteDesktopRecoverySnapshot;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::target_observer::{
    observe_bound_session_target_once, sample_platform_target_observations,
    PlatformTargetObservationSample,
};
use crate::daemon::plugins::remote_desktop::target_tracking::TargetObservation;
use crate::daemon::plugins::remote_desktop::transport::RemoteDesktopTransportManager;

const TARGET_MONITOR_INTERVAL: Duration = Duration::from_millis(250);
const TARGET_MONITOR_SUPERVISOR_INTERVAL: Duration = Duration::from_millis(25);
const TARGET_MONITOR_RETRY_BASE: Duration = Duration::from_millis(50);
const TARGET_MONITOR_RETRY_MAX: Duration = Duration::from_secs(2);
const TARGET_MONITOR_PROVIDER_DEADLINE: Duration = Duration::from_secs(1);
const TARGET_MONITOR_FAILURE_BUDGET: u32 = 3;

pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopTargetMonitor {
    worker: Mutex<LifecycleWorker<TargetMonitorCommand>>,
    desired: Mutex<HashSet<String>>,
    generation: Arc<AtomicU64>,
    snapshot_executor: Arc<TargetSnapshotDeadlineExecutor>,
    provider_deadline: Duration,
}

#[derive(Debug, Clone)]
enum TargetMonitorCommand {
    Track {
        session_id: String,
    },
    Cancel {
        session_id: String,
    },
    #[cfg(test)]
    CrashGeneration,
    Shutdown,
}

#[derive(Debug)]
enum TargetMonitorGenerationEvent {
    PollSucceeded,
    PollFailed { detail: String },
}

struct TargetMonitorGeneration {
    id: u64,
    tx: Sender<TargetMonitorCommand>,
    events: Receiver<TargetMonitorGenerationEvent>,
    join: JoinHandle<()>,
    failure_detail: Option<String>,
}

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

struct TargetSnapshotDeadlineExecutor {
    sampler: Arc<dyn TargetObservationSampler>,
    in_flight: Mutex<Option<InFlightTargetSnapshot>>,
    request_sequence: AtomicU64,
}

struct InFlightTargetSnapshot {
    request_id: u64,
    owner_generation: u64,
    result_rx: Receiver<PlatformTargetObservationSample>,
    _join: JoinHandle<()>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TargetSnapshotDeadlineError {
    DeadlineExceeded {
        request_id: u64,
        owner_generation: u64,
    },
    SpawnFailed(String),
    WorkerFailed {
        request_id: u64,
        owner_generation: u64,
    },
}

impl std::fmt::Display for TargetSnapshotDeadlineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeadlineExceeded {
                request_id,
                owner_generation,
            } => write!(
                formatter,
                "host target snapshot request {request_id} owned by generation {owner_generation} exceeded deadline"
            ),
            Self::SpawnFailed(detail) => {
                write!(formatter, "host target snapshot worker spawn failed: {detail}")
            }
            Self::WorkerFailed {
                request_id,
                owner_generation,
            } => write!(
                formatter,
                "host target snapshot request {request_id} owned by generation {owner_generation} failed"
            ),
        }
    }
}

trait TargetMediaSourceStopper {
    fn stop_endpoint_if_epoch(&self, session_id: &str, epoch: TransportEpoch) -> bool;
}

impl TargetMediaSourceStopper for RemoteDesktopTransportManager {
    fn stop_endpoint_if_epoch(&self, session_id: &str, epoch: TransportEpoch) -> bool {
        RemoteDesktopTransportManager::stop_endpoint_if_epoch(self, session_id, epoch)
    }
}

impl TargetSnapshotDeadlineExecutor {
    fn new(sampler: Arc<dyn TargetObservationSampler>) -> Self {
        Self {
            sampler,
            in_flight: Mutex::new(None),
            request_sequence: AtomicU64::new(0),
        }
    }

    fn sample_for_generation(
        &self,
        generation: u64,
        timeout: Duration,
    ) -> Result<PlatformTargetObservationSample, TargetSnapshotDeadlineError> {
        let deadline = Instant::now() + timeout;
        loop {
            let mut in_flight = match self.in_flight.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            if in_flight.is_none() {
                *in_flight = Some(self.spawn_request(generation)?);
            }
            let request = in_flight
                .as_mut()
                .expect("target snapshot request exists after spawn");
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(TargetSnapshotDeadlineError::DeadlineExceeded {
                    request_id: request.request_id,
                    owner_generation: request.owner_generation,
                });
            }
            match request.result_rx.recv_timeout(remaining) {
                Ok(sample) => {
                    let completed = in_flight
                        .take()
                        .expect("completed target snapshot request exists");
                    if completed.owner_generation != generation {
                        continue;
                    }
                    return Ok(sample);
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(TargetSnapshotDeadlineError::DeadlineExceeded {
                        request_id: request.request_id,
                        owner_generation: request.owner_generation,
                    });
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let failed = in_flight
                        .take()
                        .expect("failed target snapshot request exists");
                    if failed.owner_generation != generation {
                        continue;
                    }
                    return Err(TargetSnapshotDeadlineError::WorkerFailed {
                        request_id: failed.request_id,
                        owner_generation: failed.owner_generation,
                    });
                }
            }
        }
    }

    fn spawn_request(
        &self,
        generation: u64,
    ) -> Result<InFlightTargetSnapshot, TargetSnapshotDeadlineError> {
        let request_id = self.request_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let sampler = Arc::clone(&self.sampler);
        let (result_tx, result_rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name(format!("easynet-rd-target-snapshot-{request_id}"))
            .spawn(move || {
                let sample = sampler.sample();
                let _ = result_tx.send(sample);
            })
            .map_err(|error| TargetSnapshotDeadlineError::SpawnFailed(error.to_string()))?;
        Ok(InFlightTargetSnapshot {
            request_id,
            owner_generation: generation,
            result_rx,
            _join: join,
        })
    }
}

impl RemoteDesktopTargetMonitor {
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self::with_sampler(
            Arc::new(PlatformTargetObservationSampler),
            TARGET_MONITOR_PROVIDER_DEADLINE,
        )
    }

    fn with_sampler(
        sampler: Arc<dyn TargetObservationSampler>,
        provider_deadline: Duration,
    ) -> Self {
        Self {
            worker: Mutex::new(LifecycleWorker::new()),
            desired: Mutex::new(HashSet::new()),
            generation: Arc::new(AtomicU64::new(0)),
            snapshot_executor: Arc::new(TargetSnapshotDeadlineExecutor::new(sampler)),
            provider_deadline,
        }
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn with_sampler_for_test(
        sampler: Arc<dyn TargetObservationSampler>,
        provider_deadline: Duration,
    ) -> Self {
        Self::with_sampler(sampler, provider_deadline)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn track(
        &self,
        plugin: &Arc<RemoteDesktopPlugin>,
        session_id: String,
    ) -> anyhow::Result<()> {
        if session_id.is_empty() {
            return Ok(());
        }
        self.desired().insert(session_id.clone());
        let command = TargetMonitorCommand::Track { session_id };
        let tx = self.ensure_worker(plugin)?;
        let command = match tx.send(command) {
            Ok(()) => return Ok(()),
            Err(error) => error.0,
        };

        if let TargetMonitorCommand::Track { .. } = command {
            let tx = self.restart_supervisor(plugin)?;
            drop(tx);
        }
        Ok(())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn cancel(&self, session_id: &str) {
        self.desired().remove(session_id);
        let tx = self.worker().sender();
        if let Some(tx) = tx {
            let _ = tx.send(TargetMonitorCommand::Cancel {
                session_id: session_id.to_string(),
            });
        }
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn desired_sessions_for_test(
        &self,
    ) -> Vec<String> {
        let mut sessions: Vec<_> = self.desired().iter().cloned().collect();
        sessions.sort();
        sessions
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn generation_for_test(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn crash_generation_for_test(
        &self,
    ) -> anyhow::Result<()> {
        let tx = self
            .worker()
            .sender()
            .ok_or_else(|| anyhow::anyhow!("target monitor supervisor is not running"))?;
        tx.send(TargetMonitorCommand::CrashGeneration)
            .map_err(|_| anyhow::anyhow!("target monitor supervisor command channel closed"))
    }

    fn ensure_worker(
        &self,
        plugin: &Arc<RemoteDesktopPlugin>,
    ) -> anyhow::Result<Sender<TargetMonitorCommand>> {
        let mut worker = self.worker();
        if let Some(tx) = worker.sender() {
            return Ok(tx);
        }
        let initial_tracked = self.desired_snapshot();
        let generation = Arc::clone(&self.generation);
        let snapshot_executor = Arc::clone(&self.snapshot_executor);
        let provider_deadline = self.provider_deadline;
        worker
            .start(|| {
                spawn_target_monitor_supervisor(
                    Arc::downgrade(plugin),
                    initial_tracked,
                    generation,
                    snapshot_executor,
                    provider_deadline,
                )
            })
            .map_err(|err| anyhow::anyhow!("spawn remote desktop target monitor: {err}"))
    }

    fn restart_supervisor(
        &self,
        plugin: &Arc<RemoteDesktopPlugin>,
    ) -> anyhow::Result<Sender<TargetMonitorCommand>> {
        let mut worker = self.worker();
        let initial_tracked = self.desired_snapshot();
        let generation = Arc::clone(&self.generation);
        let snapshot_executor = Arc::clone(&self.snapshot_executor);
        let provider_deadline = self.provider_deadline;
        worker
            .start(|| {
                spawn_target_monitor_supervisor(
                    Arc::downgrade(plugin),
                    initial_tracked,
                    generation,
                    snapshot_executor,
                    provider_deadline,
                )
            })
            .map_err(|err| anyhow::anyhow!("restart remote desktop target monitor: {err}"))
    }

    fn worker(&self) -> MutexGuard<'_, LifecycleWorker<TargetMonitorCommand>> {
        match self.worker.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn desired(&self) -> MutexGuard<'_, HashSet<String>> {
        match self.desired.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn desired_snapshot(&self) -> HashSet<String> {
        self.desired().clone()
    }
}

impl Drop for RemoteDesktopTargetMonitor {
    fn drop(&mut self) {
        let worker = match self.worker.get_mut() {
            Ok(worker) => worker,
            Err(poisoned) => poisoned.into_inner(),
        };
        worker.shutdown(TargetMonitorCommand::Shutdown);
    }
}

fn spawn_target_monitor_supervisor(
    plugin: Weak<RemoteDesktopPlugin>,
    initial_tracked: HashSet<String>,
    generation_counter: Arc<AtomicU64>,
    snapshot_executor: Arc<TargetSnapshotDeadlineExecutor>,
    provider_deadline: Duration,
) -> std::io::Result<(Sender<TargetMonitorCommand>, JoinHandle<()>)> {
    let (tx, rx) = mpsc::channel();
    let join = thread::Builder::new()
        .name("easynet-rd-target-supervisor".into())
        .spawn(move || {
            run_target_monitor_supervisor(
                plugin,
                rx,
                initial_tracked,
                generation_counter,
                snapshot_executor,
                provider_deadline,
            )
        })?;
    Ok((tx, join))
}

fn run_target_monitor_supervisor(
    plugin: Weak<RemoteDesktopPlugin>,
    rx: Receiver<TargetMonitorCommand>,
    initial_tracked: HashSet<String>,
    generation_counter: Arc<AtomicU64>,
    snapshot_executor: Arc<TargetSnapshotDeadlineExecutor>,
    provider_deadline: Duration,
) {
    let mut tracked = initial_tracked;
    let mut consecutive_failures = 0_u32;
    let mut generation = None;

    loop {
        if generation.is_none() {
            match spawn_target_monitor_generation(
                plugin.clone(),
                tracked.clone(),
                &generation_counter,
                Arc::clone(&snapshot_executor),
                provider_deadline,
            ) {
                Ok(next) => {
                    generation = Some(next);
                    continue;
                }
                Err(error) => {
                    if plugin.upgrade().is_none() {
                        return;
                    }
                    let attempted_generation = generation_counter.load(Ordering::Acquire);
                    let retry_after = record_target_monitor_failure(
                        &plugin,
                        &tracked,
                        &mut consecutive_failures,
                        attempted_generation,
                        &format!("spawn failed: {error}"),
                    );
                    if !wait_for_target_monitor_retry(&rx, retry_after, &mut tracked) {
                        return;
                    }
                    continue;
                }
            }
        }

        let current = generation
            .as_mut()
            .expect("target monitor generation exists after spawn branch");
        while let Ok(event) = current.events.try_recv() {
            match event {
                TargetMonitorGenerationEvent::PollSucceeded => {
                    consecutive_failures = 0;
                    current.failure_detail = None;
                }
                TargetMonitorGenerationEvent::PollFailed { detail } => {
                    current.failure_detail = Some(detail)
                }
            }
        }

        if current.join.is_finished() {
            let mut failed = generation
                .take()
                .expect("finished target monitor generation exists");
            let failed_generation = failed.id;
            let panicked = failed.join.join().is_err();
            while let Ok(event) = failed.events.try_recv() {
                match event {
                    TargetMonitorGenerationEvent::PollSucceeded => {
                        consecutive_failures = 0;
                        failed.failure_detail = None;
                    }
                    TargetMonitorGenerationEvent::PollFailed { detail } => {
                        failed.failure_detail = Some(detail)
                    }
                }
            }
            if plugin.upgrade().is_none() {
                return;
            }
            let retry_after = record_target_monitor_failure(
                &plugin,
                &tracked,
                &mut consecutive_failures,
                failed_generation,
                if let Some(detail) = failed.failure_detail.as_deref() {
                    detail
                } else if panicked {
                    "worker panicked"
                } else {
                    "worker exited unexpectedly"
                },
            );
            if !wait_for_target_monitor_retry(&rx, retry_after, &mut tracked) {
                return;
            }
            continue;
        }

        match rx.recv_timeout(TARGET_MONITOR_SUPERVISOR_INTERVAL) {
            Ok(command) => {
                let tx = &generation
                    .as_ref()
                    .expect("target monitor generation exists while forwarding command")
                    .tx;
                if !apply_supervisor_command(command, &mut tracked, Some(tx)) {
                    let current = generation
                        .take()
                        .expect("target monitor generation exists during shutdown");
                    let _ = current.join.join();
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                let current = generation
                    .take()
                    .expect("target monitor generation exists during disconnect");
                let _ = current.tx.send(TargetMonitorCommand::Shutdown);
                let _ = current.join.join();
                return;
            }
        }
    }
}

fn record_target_monitor_failure(
    plugin: &Weak<RemoteDesktopPlugin>,
    tracked: &HashSet<String>,
    consecutive_failures: &mut u32,
    failed_generation: u64,
    detail: &str,
) -> Duration {
    *consecutive_failures = consecutive_failures.saturating_add(1);
    let retry_after = target_monitor_retry_after(*consecutive_failures);
    eprintln!(
        "[remote-desktop] target monitor generation {failed_generation} failed ({detail}); restarting after {}ms",
        retry_after.as_millis()
    );
    if *consecutive_failures == TARGET_MONITOR_FAILURE_BUDGET {
        mark_target_monitor_unhealthy(plugin, tracked, failed_generation);
    }
    retry_after
}

fn wait_for_target_monitor_retry(
    rx: &Receiver<TargetMonitorCommand>,
    retry_after: Duration,
    tracked: &mut HashSet<String>,
) -> bool {
    let deadline = Instant::now() + retry_after;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }
        match rx.recv_timeout(remaining) {
            Ok(command) => {
                if !apply_supervisor_command(command, tracked, None) {
                    return false;
                }
            }
            Err(RecvTimeoutError::Timeout) => return true,
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
}

fn spawn_target_monitor_generation(
    plugin: Weak<RemoteDesktopPlugin>,
    initial_tracked: HashSet<String>,
    generation_counter: &AtomicU64,
    snapshot_executor: Arc<TargetSnapshotDeadlineExecutor>,
    provider_deadline: Duration,
) -> std::io::Result<TargetMonitorGeneration> {
    let id = generation_counter.load(Ordering::Acquire).saturating_add(1);
    let (tx, rx) = mpsc::channel();
    let (event_tx, events) = mpsc::channel();
    let join = thread::Builder::new()
        .name(format!("easynet-rd-target-monitor-{id}"))
        .spawn(move || {
            run_target_monitor_generation(
                id,
                plugin,
                rx,
                event_tx,
                initial_tracked,
                snapshot_executor,
                provider_deadline,
            )
        })?;
    generation_counter.store(id, Ordering::Release);
    Ok(TargetMonitorGeneration {
        id,
        tx,
        events,
        join,
        failure_detail: None,
    })
}

fn run_target_monitor_generation(
    generation: u64,
    plugin: Weak<RemoteDesktopPlugin>,
    rx: Receiver<TargetMonitorCommand>,
    event_tx: Sender<TargetMonitorGenerationEvent>,
    initial_tracked: HashSet<String>,
    snapshot_executor: Arc<TargetSnapshotDeadlineExecutor>,
    provider_deadline: Duration,
) {
    let mut tracked = initial_tracked;
    loop {
        if tracked.is_empty() {
            match rx.recv() {
                Ok(command) => {
                    if !apply_generation_command(command, &mut tracked) {
                        return;
                    }
                }
                Err(_) => return,
            }
            continue;
        }

        match rx.recv_timeout(TARGET_MONITOR_INTERVAL) {
            Ok(command) => {
                if !apply_generation_command(command, &mut tracked) {
                    return;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }

        match poll_tracked_sessions(
            generation,
            &plugin,
            &mut tracked,
            &snapshot_executor,
            provider_deadline,
        ) {
            Ok(true) => {
                let _ = event_tx.send(TargetMonitorGenerationEvent::PollSucceeded);
            }
            Ok(false) => return,
            Err(error) => {
                let _ = event_tx.send(TargetMonitorGenerationEvent::PollFailed {
                    detail: error.to_string(),
                });
                return;
            }
        }
    }
}

fn apply_supervisor_command(
    command: TargetMonitorCommand,
    tracked: &mut HashSet<String>,
    generation_tx: Option<&Sender<TargetMonitorCommand>>,
) -> bool {
    match &command {
        TargetMonitorCommand::Track { session_id } => {
            if !session_id.is_empty() {
                tracked.insert(session_id.clone());
            }
        }
        TargetMonitorCommand::Cancel { session_id } => {
            tracked.remove(session_id);
        }
        #[cfg(test)]
        TargetMonitorCommand::CrashGeneration => {}
        TargetMonitorCommand::Shutdown => {
            if let Some(tx) = generation_tx {
                let _ = tx.send(TargetMonitorCommand::Shutdown);
            }
            return false;
        }
    }
    if let Some(tx) = generation_tx {
        let _ = tx.send(command);
    }
    true
}

fn apply_generation_command(command: TargetMonitorCommand, tracked: &mut HashSet<String>) -> bool {
    match command {
        TargetMonitorCommand::Track { session_id } => {
            if !session_id.is_empty() {
                tracked.insert(session_id);
            }
            true
        }
        TargetMonitorCommand::Cancel { session_id } => {
            tracked.remove(&session_id);
            true
        }
        #[cfg(test)]
        TargetMonitorCommand::CrashGeneration => {
            panic!("injected target monitor generation failure")
        }
        TargetMonitorCommand::Shutdown => false,
    }
}

fn target_monitor_retry_after(consecutive_failures: u32) -> Duration {
    let exponent = consecutive_failures.saturating_sub(1).min(6);
    TARGET_MONITOR_RETRY_BASE
        .saturating_mul(1_u32 << exponent)
        .min(TARGET_MONITOR_RETRY_MAX)
}

fn mark_target_monitor_unhealthy(
    plugin: &Weak<RemoteDesktopPlugin>,
    tracked: &HashSet<String>,
    failed_generation: u64,
) {
    let Some(plugin) = plugin.upgrade() else {
        return;
    };
    let sessions = plugin.session_store();
    let transports = plugin.transport_manager();
    for session_id in tracked {
        let Some(inputs) = sessions.target_observation_inputs_for_session(session_id) else {
            continue;
        };
        let commit = sessions.commit_target_observation_for_session(
            session_id,
            &inputs.binding_id,
            inputs.binding_epoch,
            TargetObservation::MonitorUnavailable {
                detail: format!(
                    "target monitor generation {failed_generation} exhausted its restart budget"
                ),
                observed_at_ms: now_ms(),
            },
        );
        let state_changed = commit.as_ref().is_some_and(|commit| commit.state_changed);
        let media_source_lost = commit.and_then(|commit| commit.media_source_lost);
        stop_lost_media_source(transports.as_ref(), session_id, media_source_lost);
        if state_changed {
            persist_target_monitor_snapshot(&plugin, &sessions, session_id, "unhealthy state");
        }
    }
}

fn poll_tracked_sessions(
    generation: u64,
    plugin: &Weak<RemoteDesktopPlugin>,
    tracked: &mut HashSet<String>,
    snapshot_executor: &TargetSnapshotDeadlineExecutor,
    provider_deadline: Duration,
) -> Result<bool, TargetSnapshotDeadlineError> {
    let Some(plugin) = plugin.upgrade() else {
        return Ok(false);
    };
    let sessions = plugin.session_store();
    let transports = plugin.transport_manager();
    let provider = snapshot_executor.sample_for_generation(generation, provider_deadline)?;
    tracked.retain(|session_id| {
        let result = observe_bound_session_target_once(&sessions, session_id, &provider);
        stop_lost_media_source(transports.as_ref(), session_id, result.media_source_lost);
        if result.state_changed {
            persist_target_monitor_snapshot(&plugin, &sessions, session_id, "target observation");
        }
        result.keep_tracking
    });
    Ok(true)
}

fn persist_target_monitor_snapshot(
    plugin: &RemoteDesktopPlugin,
    sessions: &RemoteDesktopSessionStore,
    session_id: &str,
    context: &str,
) {
    let snapshot = sessions.with_sessions(|rows| {
        rows.get(session_id)
            .map(RemoteDesktopRecoverySnapshot::from_session)
            .transpose()
    });
    match snapshot {
        Ok(Some(snapshot)) => {
            if let Err(error) = plugin.persist_recovery_snapshot(&snapshot) {
                eprintln!(
                    "[remote-desktop] failed to persist target monitor {context} for {session_id}: {error}"
                );
            }
        }
        Ok(None) => {}
        Err(error) => eprintln!(
            "[remote-desktop] failed to snapshot target monitor {context} for {session_id}: {error}"
        ),
    }
}

fn stop_lost_media_source<T>(
    transports: &T,
    session_id: &str,
    media_source_lost: Option<TargetMediaSourceLost>,
) -> bool
where
    T: TargetMediaSourceStopper + ?Sized,
{
    let Some(media_source_lost) = media_source_lost else {
        return false;
    };
    transports.stop_endpoint_if_epoch(session_id, media_source_lost.transport_epoch)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex, MutexGuard};
    use std::time::{Duration, Instant};

    use crate::daemon::ability::builtins::resources::media::screen_snapshot::SyntheticScreenBackend;
    use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
    use crate::daemon::plugins::remote_desktop::session::{
        RemoteDesktopSession, TargetMediaSourceLost,
    };
    use crate::daemon::plugins::remote_desktop::session_recovery::{
        RemoteDesktopRecoverySnapshot, RemoteDesktopRecoveryStore,
    };
    use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
    use crate::daemon::plugins::remote_desktop::target::TargetResolutionError;
    use crate::daemon::plugins::remote_desktop::target_monitor::{
        apply_generation_command, stop_lost_media_source, target_monitor_retry_after,
        RemoteDesktopTargetMonitor, TargetMediaSourceStopper, TargetMonitorCommand,
        TargetObservationSampler, TargetSnapshotDeadlineError, TargetSnapshotDeadlineExecutor,
        TARGET_MONITOR_RETRY_MAX,
    };
    use crate::daemon::plugins::remote_desktop::target_observer::{
        sample_platform_target_observations, PlatformTargetObservationSample,
    };
    use crate::daemon::plugins::remote_desktop::test_support::{
        test_runtime_limits, test_session_init, TestRemoteAppTargetBindingVerifier,
    };

    #[derive(Default)]
    struct RecordingStopper {
        calls: Mutex<Vec<(String, TransportEpoch)>>,
        stopped: bool,
    }

    struct BlockingFirstTargetSampler {
        calls: AtomicUsize,
        released: Mutex<bool>,
        release_signal: Condvar,
    }

    impl BlockingFirstTargetSampler {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                released: Mutex::new(false),
                release_signal: Condvar::new(),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Acquire)
        }

        fn release(&self) {
            let mut released = self
                .released
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *released = true;
            self.release_signal.notify_all();
        }
    }

    impl TargetObservationSampler for BlockingFirstTargetSampler {
        fn sample(&self) -> PlatformTargetObservationSample {
            let call = self.calls.fetch_add(1, Ordering::AcqRel);
            if call == 0 {
                let mut released = self
                    .released
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                while !*released {
                    released = self
                        .release_signal
                        .wait(released)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            }
            sample_platform_target_observations()
        }
    }

    impl RecordingStopper {
        fn calls(&self) -> MutexGuard<'_, Vec<(String, TransportEpoch)>> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        }
    }

    impl TargetMediaSourceStopper for RecordingStopper {
        fn stop_endpoint_if_epoch(&self, session_id: &str, epoch: TransportEpoch) -> bool {
            self.calls().push((session_id.to_string(), epoch));
            self.stopped
        }
    }

    #[test]
    fn target_loss_poll_result_stops_endpoint_by_epoch() {
        let stopper = RecordingStopper {
            calls: Mutex::new(Vec::new()),
            stopped: true,
        };
        let epoch = TransportEpoch::new(42);
        let stopped = stop_lost_media_source(
            &stopper,
            "rd-target-loss",
            Some(TargetMediaSourceLost {
                transport_epoch: epoch,
                reason: TargetResolutionError::TargetNotFound,
            }),
        );

        assert!(stopped);
        assert_eq!(
            stopper.calls().as_slice(),
            &[("rd-target-loss".into(), epoch)]
        );
    }

    #[test]
    fn healthy_poll_result_does_not_touch_transport_manager() {
        let stopper = RecordingStopper::default();

        assert!(!stop_lost_media_source(&stopper, "rd-healthy", None));
        assert!(stopper.calls().is_empty());
    }

    #[test]
    fn target_monitor_command_state_machine_tracks_cancels_and_shuts_down() {
        let mut tracked = HashSet::<String>::new();

        assert!(apply_generation_command(
            TargetMonitorCommand::Track {
                session_id: String::new(),
            },
            &mut tracked,
        ));
        assert!(
            tracked.is_empty(),
            "empty session ids must not enter target tracking"
        );

        assert!(apply_generation_command(
            TargetMonitorCommand::Track {
                session_id: "rd-target-a".into(),
            },
            &mut tracked,
        ));
        assert!(apply_generation_command(
            TargetMonitorCommand::Track {
                session_id: "rd-target-a".into(),
            },
            &mut tracked,
        ));
        assert_eq!(
            tracked.iter().cloned().collect::<Vec<_>>(),
            vec!["rd-target-a".to_string()],
            "target monitor tracking must be idempotent per session id"
        );

        assert!(apply_generation_command(
            TargetMonitorCommand::Track {
                session_id: "rd-target-b".into(),
            },
            &mut tracked,
        ));
        assert_eq!(tracked.len(), 2);

        assert!(apply_generation_command(
            TargetMonitorCommand::Cancel {
                session_id: "rd-target-a".into(),
            },
            &mut tracked,
        ));
        assert!(!tracked.contains("rd-target-a"));
        assert!(tracked.contains("rd-target-b"));

        assert!(!apply_generation_command(
            TargetMonitorCommand::Shutdown,
            &mut tracked
        ));
    }

    #[test]
    fn target_monitor_retry_backoff_reaches_and_stays_at_cap() {
        assert_eq!(target_monitor_retry_after(1), Duration::from_millis(50));
        assert_eq!(target_monitor_retry_after(2), Duration::from_millis(100));
        assert_eq!(target_monitor_retry_after(6), Duration::from_millis(1_600));
        assert_eq!(target_monitor_retry_after(7), TARGET_MONITOR_RETRY_MAX);
        assert_eq!(
            target_monitor_retry_after(u32::MAX),
            TARGET_MONITOR_RETRY_MAX
        );
    }

    #[test]
    fn snapshot_deadline_fences_late_result_and_bounds_native_call_count() {
        let sampler = Arc::new(BlockingFirstTargetSampler::new());
        let sampler_trait: Arc<dyn TargetObservationSampler> = sampler.clone();
        let executor = Arc::new(TargetSnapshotDeadlineExecutor::new(sampler_trait));

        let first = executor.sample_for_generation(1, Duration::from_millis(20));
        assert!(matches!(
            first,
            Err(TargetSnapshotDeadlineError::DeadlineExceeded {
                owner_generation: 1,
                ..
            })
        ));
        assert_eq!(sampler.calls(), 1);

        let next_executor = Arc::clone(&executor);
        let next = std::thread::spawn(move || {
            next_executor.sample_for_generation(2, Duration::from_secs(1))
        });
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(
            sampler.calls(),
            1,
            "a replacement generation must wait on the existing native call"
        );

        sampler.release();
        next.join()
            .expect("replacement generation joins")
            .expect("replacement generation starts a fresh snapshot after fencing stale result");
        assert_eq!(
            sampler.calls(),
            2,
            "the late generation-1 sample must be discarded before one fresh generation-2 call"
        );
    }

    #[test]
    fn supervisor_restarts_panicked_generation_without_a_new_track_command() {
        let plugin = RemoteDesktopPlugin::with_target_binding_verifier(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
        );
        RemoteDesktopPlugin::track_target_for_test(&plugin, "rd-supervised-generation")
            .expect("target tracking starts the supervisor");
        let first_generation = wait_for_generation_after(&plugin, 0);

        plugin
            .crash_target_monitor_generation_for_test()
            .expect("fault injection reaches the supervisor");
        let restarted_generation = wait_for_generation_after(&plugin, first_generation);

        assert!(restarted_generation > first_generation);
        assert_eq!(
            plugin.target_monitor_desired_sessions_for_test(),
            vec!["rd-supervised-generation".to_string()],
            "the supervisor must seed the new generation from desired state"
        );
    }

    #[test]
    fn restart_budget_exhaustion_marks_and_persists_target_unavailable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let recovery = Arc::new(RemoteDesktopRecoveryStore::new(temp.path().to_path_buf()));
        let plugin = RemoteDesktopPlugin::with_recovery_store_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
            Arc::clone(&recovery),
        );
        let session_id = "rd-monitor-budget";
        let session = RemoteDesktopSession::new(test_session_init(
            session_id,
            "easynet:///r/acme/resource/display.monitor-budget",
            vec!["webrtc".into()],
        ));
        plugin.session_store().with_sessions(|sessions| {
            sessions.insert(session_id.to_string(), session);
        });
        RemoteDesktopPlugin::track_target_for_test(&plugin, session_id)
            .expect("target tracking starts the supervisor");

        let mut generation = wait_for_generation_after(&plugin, 0);
        for failure in 1..=3 {
            plugin
                .crash_target_monitor_generation_for_test()
                .expect("fault injection reaches active generation");
            if failure < 3 {
                generation = wait_for_generation_after(&plugin, generation);
            }
        }

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let lost = plugin.session_store().with_sessions(|sessions| {
                sessions
                    .get(session_id)
                    .is_some_and(|session| session.target_tracking_state()["status"] == "lost")
            });
            if lost {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "restart budget did not commit target unavailable"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        let snapshot = wait_for_recovery_snapshot(&recovery, session_id);
        let recovered = RemoteDesktopSession::rehydrate(&snapshot)
            .expect("persisted unavailable target rehydrates");
        assert_eq!(recovered.target_tracking_state()["status"], "lost");
        assert_eq!(recovered.target_tracking_state()["input_enabled"], false);
    }

    #[test]
    fn provider_hang_exhausts_budget_without_spawning_unbounded_native_calls() {
        let temp = tempfile::tempdir().expect("tempdir");
        let recovery = Arc::new(RemoteDesktopRecoveryStore::new(temp.path().to_path_buf()));
        let sampler = Arc::new(BlockingFirstTargetSampler::new());
        let sampler_trait: Arc<dyn TargetObservationSampler> = sampler.clone();
        let monitor = Arc::new(RemoteDesktopTargetMonitor::with_sampler_for_test(
            sampler_trait,
            Duration::from_millis(25),
        ));
        let plugin = RemoteDesktopPlugin::with_target_monitor_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
            Arc::clone(&recovery),
            monitor,
        );
        let session_id = "rd-monitor-provider-hang";
        let session = RemoteDesktopSession::new(test_session_init(
            session_id,
            "easynet:///r/acme/resource/display.monitor-provider-hang",
            vec!["webrtc".into()],
        ));
        plugin.session_store().with_sessions(|sessions| {
            sessions.insert(session_id.to_string(), session);
        });
        RemoteDesktopPlugin::track_target_for_test(&plugin, session_id)
            .expect("target tracking starts the supervisor");

        wait_for_target_status(&plugin, session_id, "lost", Duration::from_secs(3));
        assert_eq!(
            sampler.calls(),
            1,
            "three timed-out generations must share one bounded native call"
        );
        let snapshot = wait_for_recovery_snapshot(&recovery, session_id);
        let recovered = RemoteDesktopSession::rehydrate(&snapshot)
            .expect("hung-provider unavailable state rehydrates");
        assert_eq!(recovered.target_tracking_state()["status"], "lost");
        assert_eq!(recovered.target_tracking_state()["input_enabled"], false);

        let shutdown_started = Instant::now();
        drop(plugin);
        assert!(
            shutdown_started.elapsed() < Duration::from_millis(500),
            "plugin shutdown must not join the blocked native provider call"
        );
        sampler.release();
    }

    fn wait_for_generation_after(plugin: &RemoteDesktopPlugin, previous: u64) -> u64 {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let current = plugin.target_monitor_generation_for_test();
            if current > previous {
                return current;
            }
            assert!(
                Instant::now() < deadline,
                "target monitor generation did not advance after injected failure"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_target_status(
        plugin: &RemoteDesktopPlugin,
        session_id: &str,
        expected: &str,
        timeout: Duration,
    ) {
        let deadline = Instant::now() + timeout;
        loop {
            let matches = plugin.session_store().with_sessions(|sessions| {
                sessions
                    .get(session_id)
                    .is_some_and(|session| session.target_tracking_state()["status"] == expected)
            });
            if matches {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "target monitor did not reach expected status {expected}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_recovery_snapshot(
        recovery: &RemoteDesktopRecoveryStore,
        session_id: &str,
    ) -> RemoteDesktopRecoverySnapshot {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(snapshot) = recovery.load(session_id).expect("load recovery snapshot") {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "target monitor did not persist unavailable snapshot"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
