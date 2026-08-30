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
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::daemon::plugins::remote_desktop::lease_monitor::RemoteDesktopLeaseMonitor;
use crate::daemon::plugins::remote_desktop::lifecycle_worker::LifecycleWorker;
use crate::daemon::plugins::remote_desktop::relay_lease::RemoteDesktopRelayLeaseProvider;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::{now_ms, TargetMediaSourceLost};
use crate::daemon::plugins::remote_desktop::session_lifecycle::{
    commit_prepared_closing_checkpoint, settle_session_transports_and_finish,
    DurableClosingCheckpoint, RetiredSessionTransports,
};
use crate::daemon::plugins::remote_desktop::session_recovery::{
    RemoteDesktopRecoverySnapshot, RemoteDesktopRecoveryStore,
};
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::target_observer::observe_bound_session_target_once_with_closing_checkpoint;
#[cfg(test)]
use crate::daemon::plugins::remote_desktop::target_observer::PlatformTargetObservationSample;
#[cfg(test)]
use crate::daemon::plugins::remote_desktop::target_snapshot::TargetObservationSampler;
use crate::daemon::plugins::remote_desktop::target_snapshot::{
    TargetSnapshotDeadlineError, TargetSnapshotDeadlineExecutor,
};
use crate::daemon::plugins::remote_desktop::target_tracking::TargetObservation;
use crate::daemon::plugins::remote_desktop::transport::{
    RemoteDesktopTransportManager, TransportSettlementStatus,
};

const TARGET_MONITOR_INTERVAL: Duration = Duration::from_millis(250);
const TARGET_MONITOR_SUPERVISOR_INTERVAL: Duration = Duration::from_millis(25);
const TARGET_MONITOR_RETRY_BASE: Duration = Duration::from_millis(50);
const TARGET_MONITOR_RETRY_MAX: Duration = Duration::from_secs(2);
const TARGET_MONITOR_PROVIDER_DEADLINE: Duration = Duration::from_secs(1);
const TARGET_MONITOR_FAILURE_BUDGET: u32 = 3;

#[cfg(test)]
struct StableTargetObservationSampler;

#[cfg(test)]
impl TargetObservationSampler for StableTargetObservationSampler {
    fn sample(&self) -> PlatformTargetObservationSample {
        PlatformTargetObservationSample::no_change_for_test()
    }
}

pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopTargetMonitor {
    worker: Mutex<LifecycleWorker<TargetMonitorCommand>>,
    desired: Arc<Mutex<HashSet<String>>>,
    generation: Arc<AtomicU64>,
    snapshot_executor: Arc<TargetSnapshotDeadlineExecutor>,
    provider_deadline: Duration,
}

/// Runtime components required by target observation workers.
///
/// This context deliberately does not retain `RemoteDesktopPlugin`. A worker
/// may be the final thread using these components while the plugin owner is
/// being dropped. Retaining the aggregate from that worker would let the last
/// `Arc<RemoteDesktopPlugin>` drop on the generation thread, whose target
/// monitor then joins the supervisor while the supervisor joins the
/// generation: a circular join. Component ownership keeps shutdown acyclic.
#[derive(Clone)]
struct TargetMonitorRuntimeContext {
    sessions: Arc<RemoteDesktopSessionStore>,
    transports: Arc<RemoteDesktopTransportManager>,
    recovery: Arc<RemoteDesktopRecoveryStore>,
    leases: Arc<RemoteDesktopLeaseMonitor>,
    relay_lease_provider: Arc<dyn RemoteDesktopRelayLeaseProvider>,
    desired: Arc<Mutex<HashSet<String>>>,
}

impl TargetMonitorRuntimeContext {
    fn from_plugin(plugin: &RemoteDesktopPlugin, desired: Arc<Mutex<HashSet<String>>>) -> Self {
        Self {
            sessions: plugin.session_store(),
            transports: plugin.transport_manager(),
            recovery: plugin.recovery_store(),
            leases: plugin.lease_monitor(),
            relay_lease_provider: plugin.relay_lease_provider(),
            desired,
        }
    }

    fn cancel_termination_inventory(&self, session_id: &str) {
        match self.desired.lock() {
            Ok(mut desired) => {
                desired.remove(session_id);
            }
            Err(poisoned) => {
                poisoned.into_inner().remove(session_id);
            }
        }
        self.leases.cancel(session_id);
    }
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
    recovery_from_generation: Option<u64>,
}

trait TargetMediaSourceStopper {
    fn stop_and_settle_endpoint_if_epoch(&self, session_id: &str, epoch: TransportEpoch) -> bool;
}

impl TargetMediaSourceStopper for RemoteDesktopTransportManager {
    fn stop_and_settle_endpoint_if_epoch(&self, session_id: &str, epoch: TransportEpoch) -> bool {
        let Some(endpoint) = self.take_endpoint_if_epoch_for_settlement(session_id, epoch) else {
            return false;
        };
        endpoint.settle()
    }
}

impl RemoteDesktopTargetMonitor {
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self::with_sampler(
            Arc::new(TargetSnapshotDeadlineExecutor::platform()),
            TARGET_MONITOR_PROVIDER_DEADLINE,
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn snapshot_executor(
        &self,
    ) -> Arc<TargetSnapshotDeadlineExecutor> {
        Arc::clone(&self.snapshot_executor)
    }

    fn with_sampler(
        snapshot_executor: Arc<TargetSnapshotDeadlineExecutor>,
        provider_deadline: Duration,
    ) -> Self {
        Self {
            worker: Mutex::new(LifecycleWorker::new()),
            desired: Arc::new(Mutex::new(HashSet::new())),
            generation: Arc::new(AtomicU64::new(0)),
            snapshot_executor,
            provider_deadline,
        }
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn with_sampler_for_test(
        sampler: Arc<dyn TargetObservationSampler>,
        provider_deadline: Duration,
    ) -> Self {
        Self::with_sampler(
            Arc::new(TargetSnapshotDeadlineExecutor::new(sampler)),
            provider_deadline,
        )
    }

    /// Build a deterministic monitor for aggregate/handler unit tests.
    ///
    /// Those tests inject a synthetic target-binding verifier, so consulting
    /// the real host topology would combine contradictory authorities and make
    /// session state depend on the developer machine. Dedicated target-monitor
    /// tests inject explicit samplers through `with_sampler_for_test` instead.
    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn stable_for_test() -> Self {
        Self::with_sampler(
            Arc::new(TargetSnapshotDeadlineExecutor::new(Arc::new(
                StableTargetObservationSampler,
            ))),
            TARGET_MONITOR_PROVIDER_DEADLINE,
        )
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
        let runtime = TargetMonitorRuntimeContext::from_plugin(plugin, Arc::clone(&self.desired));
        worker
            .start(|| {
                spawn_target_monitor_supervisor(
                    runtime,
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
        let runtime = TargetMonitorRuntimeContext::from_plugin(plugin, Arc::clone(&self.desired));
        worker
            .start(|| {
                spawn_target_monitor_supervisor(
                    runtime,
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
    runtime: TargetMonitorRuntimeContext,
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
                runtime,
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
    runtime: TargetMonitorRuntimeContext,
    rx: Receiver<TargetMonitorCommand>,
    initial_tracked: HashSet<String>,
    generation_counter: Arc<AtomicU64>,
    snapshot_executor: Arc<TargetSnapshotDeadlineExecutor>,
    provider_deadline: Duration,
) {
    let mut tracked = initial_tracked;
    let mut consecutive_failures = 0_u32;
    let mut generation = None;
    let mut pending_failed_generation = None;

    loop {
        if generation.is_none() {
            match spawn_target_monitor_generation(
                runtime.clone(),
                tracked.clone(),
                &generation_counter,
                Arc::clone(&snapshot_executor),
                provider_deadline,
            ) {
                Ok(mut next) => {
                    if let Some(failed_generation) = pending_failed_generation.take() {
                        record_target_monitor_worker_restarted(
                            &runtime,
                            &tracked,
                            failed_generation,
                            next.id,
                        );
                        next.recovery_from_generation = Some(failed_generation);
                    }
                    generation = Some(next);
                    continue;
                }
                Err(error) => {
                    let attempted_generation = generation_counter.load(Ordering::Acquire);
                    let retry_after = record_target_monitor_failure(
                        &runtime,
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
            apply_target_monitor_generation_event(
                &runtime,
                &tracked,
                current,
                event,
                &mut consecutive_failures,
            );
        }

        if current.join.is_finished() {
            let mut failed = generation
                .take()
                .expect("finished target monitor generation exists");
            let failed_generation = failed.id;
            while let Ok(event) = failed.events.try_recv() {
                apply_target_monitor_generation_event(
                    &runtime,
                    &tracked,
                    &mut failed,
                    event,
                    &mut consecutive_failures,
                );
            }
            let panicked = failed.join.join().is_err();
            let failure_detail = failed.failure_detail.clone().unwrap_or_else(|| {
                if panicked {
                    "worker panicked".to_string()
                } else {
                    "worker exited unexpectedly".to_string()
                }
            });
            record_target_monitor_worker_crashed(
                &runtime,
                &tracked,
                failed_generation,
                &failure_detail,
            );
            pending_failed_generation = Some(failed_generation);
            let retry_after = record_target_monitor_failure(
                &runtime,
                &tracked,
                &mut consecutive_failures,
                failed_generation,
                &failure_detail,
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

#[derive(Clone, Copy)]
enum TargetMonitorLifecycleEvent<'a> {
    WorkerCrashed {
        failed_generation: u64,
        detail: &'a str,
    },
    WorkerRestarted {
        failed_generation: u64,
        restarted_generation: u64,
    },
    MonitorRestarted {
        failed_generation: u64,
        restarted_generation: u64,
    },
}

fn record_target_monitor_worker_crashed(
    runtime: &TargetMonitorRuntimeContext,
    tracked: &HashSet<String>,
    failed_generation: u64,
    detail: &str,
) {
    persist_target_monitor_lifecycle_event(
        runtime,
        tracked,
        TargetMonitorLifecycleEvent::WorkerCrashed {
            failed_generation,
            detail,
        },
    );
}

fn record_target_monitor_worker_restarted(
    runtime: &TargetMonitorRuntimeContext,
    tracked: &HashSet<String>,
    failed_generation: u64,
    restarted_generation: u64,
) {
    persist_target_monitor_lifecycle_event(
        runtime,
        tracked,
        TargetMonitorLifecycleEvent::WorkerRestarted {
            failed_generation,
            restarted_generation,
        },
    );
}

fn record_target_monitor_restarted(
    runtime: &TargetMonitorRuntimeContext,
    tracked: &HashSet<String>,
    failed_generation: u64,
    restarted_generation: u64,
) {
    persist_target_monitor_lifecycle_event(
        runtime,
        tracked,
        TargetMonitorLifecycleEvent::MonitorRestarted {
            failed_generation,
            restarted_generation,
        },
    );
}

fn persist_target_monitor_lifecycle_event(
    runtime: &TargetMonitorRuntimeContext,
    tracked: &HashSet<String>,
    event: TargetMonitorLifecycleEvent<'_>,
) {
    let snapshots = runtime.sessions.with_sessions(|rows| {
        tracked
            .iter()
            .filter_map(|session_id| {
                let session = rows.get_mut(session_id)?;
                match event {
                    TargetMonitorLifecycleEvent::WorkerCrashed {
                        failed_generation,
                        detail,
                    } => session.record_target_monitor_worker_crashed(failed_generation, detail),
                    TargetMonitorLifecycleEvent::WorkerRestarted {
                        failed_generation,
                        restarted_generation,
                    } => session.record_target_monitor_worker_restarted(
                        failed_generation,
                        restarted_generation,
                    ),
                    TargetMonitorLifecycleEvent::MonitorRestarted {
                        failed_generation,
                        restarted_generation,
                    } => session
                        .record_target_monitor_restarted(failed_generation, restarted_generation),
                }
                Some((
                    session_id.clone(),
                    RemoteDesktopRecoverySnapshot::from_session(session),
                ))
            })
            .collect::<Vec<_>>()
    });

    for (session_id, snapshot) in snapshots {
        match snapshot {
            Ok(snapshot) => {
                if let Err(error) = runtime.recovery.save(&snapshot) {
                    eprintln!(
                        "[remote-desktop] failed to persist target monitor lifecycle event for {session_id}: {error}"
                    );
                }
            }
            Err(error) => eprintln!(
                "[remote-desktop] failed to snapshot target monitor lifecycle event for {session_id}: {error}"
            ),
        }
    }
}

fn record_target_monitor_failure(
    runtime: &TargetMonitorRuntimeContext,
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
        mark_target_monitor_unhealthy(runtime, tracked, failed_generation);
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
    runtime: TargetMonitorRuntimeContext,
    initial_tracked: HashSet<String>,
    generation_counter: &AtomicU64,
    snapshot_executor: Arc<TargetSnapshotDeadlineExecutor>,
    provider_deadline: Duration,
) -> std::io::Result<TargetMonitorGeneration> {
    let id = generation_counter
        .load(Ordering::Acquire)
        .checked_add(1)
        .ok_or_else(|| {
            std::io::Error::other("RemoteApp target monitor generation sequence exhausted")
        })?;
    let (tx, rx) = mpsc::channel();
    let (event_tx, events) = mpsc::channel();
    let join = thread::Builder::new()
        .name(format!("easynet-rd-target-monitor-{id}"))
        .spawn(move || {
            run_target_monitor_generation(
                id,
                runtime,
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
        recovery_from_generation: None,
    })
}

fn apply_target_monitor_generation_event(
    runtime: &TargetMonitorRuntimeContext,
    tracked: &HashSet<String>,
    generation: &mut TargetMonitorGeneration,
    event: TargetMonitorGenerationEvent,
    consecutive_failures: &mut u32,
) {
    match event {
        TargetMonitorGenerationEvent::PollSucceeded => {
            *consecutive_failures = 0;
            generation.failure_detail = None;
            if let Some(failed_generation) = generation.recovery_from_generation.take() {
                record_target_monitor_restarted(runtime, tracked, failed_generation, generation.id);
            }
        }
        TargetMonitorGenerationEvent::PollFailed { detail } => {
            generation.failure_detail = Some(detail);
        }
    }
}

fn run_target_monitor_generation(
    generation: u64,
    runtime: TargetMonitorRuntimeContext,
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

        #[cfg(all(feature = "remoteapp-e2e-fault-injection", unix))]
        crate::daemon::plugins::remote_desktop::e2e_fault_injection::maybe_crash_target_monitor_generation(
            generation,
            &tracked,
        );

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
            &runtime,
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
    runtime: &TargetMonitorRuntimeContext,
    tracked: &HashSet<String>,
    failed_generation: u64,
) {
    let sessions = &runtime.sessions;
    let transports = &runtime.transports;
    for session_id in tracked {
        let Some(inputs) = sessions.target_observation_inputs_for_session(session_id) else {
            continue;
        };
        let commit = sessions.commit_target_observation_for_session(
            session_id,
            &inputs.binding_id,
            inputs.binding_epoch,
            &inputs.snapshot,
            &inputs.coherence_token,
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
            persist_target_monitor_snapshot(
                &runtime.recovery,
                sessions,
                session_id,
                "unhealthy state",
            );
        }
    }
}

fn poll_tracked_sessions(
    generation: u64,
    runtime: &TargetMonitorRuntimeContext,
    tracked: &mut HashSet<String>,
    snapshot_executor: &TargetSnapshotDeadlineExecutor,
    provider_deadline: Duration,
) -> Result<bool, TargetSnapshotDeadlineError> {
    let sessions = &runtime.sessions;
    let transports = &runtime.transports;
    let provider = snapshot_executor.sample_for_generation(generation, provider_deadline)?;
    tracked.retain(|session_id| {
        let mut durable_closing_checkpoint = None;
        let result = observe_bound_session_target_once_with_closing_checkpoint(
            sessions,
            session_id,
            &provider,
            |snapshot| {
                match commit_prepared_closing_checkpoint(runtime.recovery.as_ref(), snapshot) {
                    Ok(checkpoint) => {
                        durable_closing_checkpoint = Some(checkpoint);
                        true
                    }
                    Err(error) => {
                        eprintln!(
                            "[remote-desktop] target monitor permission-revocation Closing checkpoint remains pending for {session_id}: {error}"
                        );
                        false
                    }
                }
            },
        );
        if result.permission_verification_started {
            if let Some(preview) = stop_preview_transport(sessions, transports, session_id) {
                transports.settlement_queue().enqueue(preview);
            }
            stop_lost_media_source(transports.as_ref(), session_id, result.media_source_lost);
        }
        if result.permission_revocation_started {
            persist_target_monitor_snapshot(
                runtime.recovery.as_ref(),
                sessions,
                session_id,
                "permission-revocation Closing projection",
            );
        } else if !result.permission_verification_started {
            stop_lost_media_source(transports.as_ref(), session_id, result.media_source_lost);
        }
        if result.permission_revocation_started {
            let checkpoint = durable_closing_checkpoint
                .take()
                .expect("permission revocation cannot commit without durable Closing checkpoint");
            let transports = retire_permission_revoked_transports(
                runtime,
                session_id,
                checkpoint,
            );
            let settlement = settle_session_transports_and_finish(
                runtime.transports.settlement_queue(),
                Arc::clone(sessions),
                Arc::clone(&runtime.recovery),
                Arc::clone(&runtime.relay_lease_provider),
                session_id.to_string(),
                transports,
            );
            match settlement {
                TransportSettlementStatus::Settled => {}
                TransportSettlementStatus::Pending => eprintln!(
                    "[remote-desktop] permission revocation for {session_id} continues under the session-owned settler; retaining Closing until real receipts arrive"
                ),
                TransportSettlementStatus::Failed => eprintln!(
                    "[remote-desktop] permission revocation for {session_id} received an explicit transport settlement failure; retaining Closing"
                ),
            }
        } else if result.state_changed {
            persist_target_monitor_snapshot(
                &runtime.recovery,
                sessions,
                session_id,
                "target observation",
            );
        }
        result.keep_tracking
    });
    Ok(true)
}

fn retire_permission_revoked_transports(
    runtime: &TargetMonitorRuntimeContext,
    session_id: &str,
    checkpoint: DurableClosingCheckpoint,
) -> RetiredSessionTransports {
    let operation_lock = runtime.sessions.target_operation_lock(session_id);
    let _operation = match operation_lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let preview_stop = runtime.sessions.with_sessions(|rows| {
        let session = rows
            .get_mut(session_id)
            .expect("permission-revoked Closing session remains owned");
        checkpoint.assert_matches(session_id, session);
        session.detach_preview_transport()
    });
    // Every ownership mutation below is authorized by the durable checkpoint
    // validated above while the per-session operation gate remains held.
    runtime.cancel_termination_inventory(session_id);
    if let Some(stop_tx) = preview_stop {
        let _ = stop_tx.send(true);
    }
    let diagnostic_preview = runtime.transports.take_preview_for_settlement(session_id);
    let direct_webrtc = runtime.transports.take_endpoint_for_settlement(session_id);
    RetiredSessionTransports::new(direct_webrtc, diagnostic_preview)
        .unwrap_or_else(RetiredSessionTransports::empty)
}

fn stop_preview_transport(
    sessions: &RemoteDesktopSessionStore,
    transports: &RemoteDesktopTransportManager,
    session_id: &str,
) -> Option<crate::daemon::plugins::remote_desktop::transport::RetiredDiagnosticPreview> {
    let stop_tx = sessions.with_sessions(|rows| {
        rows.get_mut(session_id)
            .and_then(|session| session.detach_preview_transport())
    });
    if let Some(stop_tx) = stop_tx {
        let _ = stop_tx.send(true);
    }
    transports.take_preview_for_settlement(session_id)
}

fn persist_target_monitor_snapshot(
    recovery: &RemoteDesktopRecoveryStore,
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
            if let Err(error) = recovery.save(&snapshot) {
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
    transports.stop_and_settle_endpoint_if_epoch(session_id, media_source_lost.transport_epoch)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard};
    use std::time::{Duration, Instant};

    use crate::daemon::ability::builtins::resources::media::screen_snapshot::SyntheticScreenBackend;
    use crate::daemon::plugins::remote_desktop::constants::REASON_TARGET_PERMISSION_REVOKED;
    use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
    use crate::daemon::plugins::remote_desktop::session::{
        now_ms, RemoteDesktopSession, TargetMediaSourceLost,
    };
    use crate::daemon::plugins::remote_desktop::session_recovery::{
        RemoteDesktopRecoverySnapshot, RemoteDesktopRecoveryStore,
    };
    use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
    use crate::daemon::plugins::remote_desktop::target::TargetResolutionError;
    use crate::daemon::plugins::remote_desktop::target_monitor::{
        apply_generation_command, stop_lost_media_source, target_monitor_retry_after,
        RemoteDesktopTargetMonitor, TargetMediaSourceStopper, TargetMonitorCommand,
        TargetMonitorRuntimeContext, TARGET_MONITOR_RETRY_MAX,
    };
    use crate::daemon::plugins::remote_desktop::target_observer::PlatformTargetObservationSample;
    use crate::daemon::plugins::remote_desktop::target_snapshot::{
        TargetObservationSampler, TargetSnapshotDeadlineError, TargetSnapshotDeadlineExecutor,
        TargetSnapshotOwner,
    };
    use crate::daemon::plugins::remote_desktop::target_tracking::TargetObservation;
    use crate::daemon::plugins::remote_desktop::test_support::{
        test_runtime_limits, test_session_init, TestRemoteAppTargetBindingVerifier,
    };
    use crate::daemon::plugins::remote_desktop::transport::PreviewTaskGroupCompletion;

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

    struct PermissionRevokedTargetSampler;

    #[derive(Default)]
    struct CountingPermissionRevokedTargetSampler {
        calls: AtomicUsize,
    }

    impl TargetObservationSampler for PermissionRevokedTargetSampler {
        fn sample(&self) -> PlatformTargetObservationSample {
            PlatformTargetObservationSample::permission_revoked(
                "injected screen-capture permission revocation",
            )
        }
    }

    impl TargetObservationSampler for CountingPermissionRevokedTargetSampler {
        fn sample(&self) -> PlatformTargetObservationSample {
            self.calls.fetch_add(1, Ordering::SeqCst);
            PlatformTargetObservationSample::permission_revoked(
                "injected screen-capture permission revocation",
            )
        }
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
            PlatformTargetObservationSample::no_change_for_test()
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
        fn stop_and_settle_endpoint_if_epoch(
            &self,
            session_id: &str,
            epoch: TransportEpoch,
        ) -> bool {
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
                owner: TargetSnapshotOwner::MonitorGeneration(1),
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
    fn input_deadline_shares_monitor_single_flight_and_fences_monitor_result() {
        let sampler = Arc::new(BlockingFirstTargetSampler::new());
        let sampler_trait: Arc<dyn TargetObservationSampler> = sampler.clone();
        let executor = TargetSnapshotDeadlineExecutor::new(sampler_trait);

        assert!(matches!(
            executor.sample_for_generation(7, Duration::from_millis(20)),
            Err(TargetSnapshotDeadlineError::DeadlineExceeded {
                owner: TargetSnapshotOwner::MonitorGeneration(7),
                ..
            })
        ));
        assert!(matches!(
            executor.sample_for_input(Duration::from_millis(20)),
            Err(TargetSnapshotDeadlineError::DeadlineExceeded {
                owner: TargetSnapshotOwner::MonitorGeneration(7),
                ..
            })
        ));
        assert_eq!(
            sampler.calls(),
            1,
            "input timeout must not create a second native call behind a hung monitor sample"
        );

        sampler.release();
        executor
            .sample_for_input(Duration::from_secs(1))
            .expect("fresh input-owned sample succeeds after stale monitor result is fenced");
        assert_eq!(
            sampler.calls(),
            2,
            "the completed monitor-owned sample must be discarded before input validation"
        );
    }

    #[test]
    fn worker_context_does_not_retain_the_plugin_aggregate() {
        let plugin = RemoteDesktopPlugin::with_target_binding_verifier(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
        );
        assert_eq!(Arc::strong_count(&plugin), 1);

        let runtime =
            TargetMonitorRuntimeContext::from_plugin(&plugin, Arc::new(Mutex::new(HashSet::new())));

        assert_eq!(
            Arc::strong_count(&plugin),
            1,
            "target monitor workers must retain components, never the aggregate plugin owner"
        );
        drop(plugin);
        assert_eq!(Arc::strong_count(&runtime.sessions), 1);
        assert_eq!(Arc::strong_count(&runtime.transports), 1);
        assert_eq!(Arc::strong_count(&runtime.recovery), 1);
    }

    #[test]
    fn supervisor_restarts_panicked_generation_without_a_new_track_command() {
        let temp = tempfile::tempdir().expect("tempdir");
        let recovery = Arc::new(RemoteDesktopRecoveryStore::new(temp.path().to_path_buf()));
        let plugin = RemoteDesktopPlugin::with_recovery_store_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
            Arc::clone(&recovery),
        );
        let session_id = "rd-supervised-generation";
        let session = RemoteDesktopSession::new(test_session_init(
            session_id,
            "easynet:///r/acme/resource/display.supervised-generation",
            vec!["webrtc".into()],
        ));
        plugin.session_store().with_sessions(|sessions| {
            sessions.insert(session_id.to_string(), session);
        });
        RemoteDesktopPlugin::track_target_for_test(&plugin, session_id)
            .expect("target tracking starts the supervisor");
        let first_generation = wait_for_generation_after(&plugin, 0);

        plugin
            .crash_target_monitor_generation_for_test()
            .expect("fault injection reaches the supervisor");
        let restarted_generation = wait_for_generation_after(&plugin, first_generation);

        assert!(restarted_generation > first_generation);
        assert_eq!(
            plugin.target_monitor_desired_sessions_for_test(),
            vec![session_id.to_string()],
            "the supervisor must seed the new generation from desired state"
        );

        let deadline = Instant::now() + Duration::from_secs(3);
        let lifecycle_events = loop {
            let events = plugin.session_store().with_sessions(|sessions| {
                sessions
                    .get(session_id)
                    .expect("tracked session remains present")
                    .events()
                    .into_iter()
                    .filter(|event| {
                        matches!(
                            event["event_type"].as_str(),
                            Some(
                                "PLUGIN_WORKER_CRASHED"
                                    | "PLUGIN_WORKER_RESTARTED"
                                    | "TARGET_MONITOR_RESTARTED"
                            )
                        )
                    })
                    .collect::<Vec<_>>()
            });
            if events.len() == 3 {
                break events;
            }
            assert!(
                Instant::now() < deadline,
                "replacement generation did not publish functional recovery"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            lifecycle_events
                .iter()
                .filter_map(|event| event["event_type"].as_str())
                .collect::<Vec<_>>(),
            vec![
                "PLUGIN_WORKER_CRASHED",
                "PLUGIN_WORKER_RESTARTED",
                "TARGET_MONITOR_RESTARTED",
            ]
        );
        assert_eq!(
            lifecycle_events[0]["payload"]["failed_generation"],
            first_generation
        );
        assert_eq!(
            lifecycle_events[2]["payload"]["restarted_generation"],
            restarted_generation
        );
        let snapshot = wait_for_recovery_snapshot(&recovery, session_id, |snapshot| {
            snapshot
                .events()
                .iter()
                .any(|event| event["event_type"] == "TARGET_MONITOR_RESTARTED")
        });
        let recovered = RemoteDesktopSession::rehydrate(&snapshot)
            .expect("worker lifecycle events persist with the session aggregate");
        assert!(recovered
            .events()
            .iter()
            .any(|event| { event["event_type"] == "TARGET_MONITOR_RESTARTED" }));
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

        let snapshot = wait_for_recovery_snapshot(&recovery, session_id, |snapshot| {
            snapshot
                .target_tracking()
                .is_some_and(|tracking| tracking["status"] == "lost")
        });
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
        let snapshot = wait_for_recovery_snapshot(&recovery, session_id, |snapshot| {
            snapshot
                .target_tracking()
                .is_some_and(|tracking| tracking["status"] == "lost")
        });
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

    #[test]
    fn permission_revocation_durably_stops_preview_and_clears_desired_tracking() {
        let temp = tempfile::tempdir().expect("tempdir");
        let recovery = Arc::new(RemoteDesktopRecoveryStore::new(temp.path().to_path_buf()));
        let monitor = Arc::new(RemoteDesktopTargetMonitor::with_sampler_for_test(
            Arc::new(PermissionRevokedTargetSampler),
            Duration::from_millis(250),
        ));
        let plugin = RemoteDesktopPlugin::with_target_monitor_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
            Arc::clone(&recovery),
            monitor,
        );
        let session_id = "rd-monitor-permission-revoked";
        let mut session = RemoteDesktopSession::new(test_session_init(
            session_id,
            "easynet:///r/acme/resource/display.monitor-permission",
            vec!["invoke_bidi".into()],
        ));
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let preview_epoch = session
            .attach_preview_transport(stop_tx.clone())
            .expect("preview attaches before permission revocation")
            .0;
        plugin.session_store().with_sessions(|sessions| {
            sessions.insert(session_id.to_string(), session);
        });
        let (preview_done_tx, preview_done_rx) = mpsc::channel();
        plugin.transport_manager().activate_preview(
            session_id.to_string(),
            preview_epoch,
            stop_tx,
            preview_done_rx,
        );
        RemoteDesktopPlugin::track_target_for_test(&plugin, session_id)
            .expect("target tracking starts the supervisor");

        let closing_deadline = Instant::now() + Duration::from_secs(2);
        while !*stop_rx.borrow_and_update() && Instant::now() < closing_deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            *stop_rx.borrow_and_update(),
            "permission verification publishes stop on its first negative sample"
        );
        preview_done_tx
            .send(PreviewTaskGroupCompletion)
            .expect("preview worker group completion is still owned");

        let revocation_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let terminating = plugin.session_store().with_sessions(|sessions| {
                sessions
                    .get(session_id)
                    .is_some_and(|session| session.is_terminating())
            });
            if terminating {
                break;
            }
            assert!(
                Instant::now() < revocation_deadline,
                "second permission denial did not confirm revocation"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        plugin.session_store().with_sessions(|sessions| {
            let session = sessions.get(session_id).expect("revoked session exists");
            assert!(session.is_terminating());
            assert!(!session.is_terminal());
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let terminal = plugin.session_store().with_sessions(|sessions| {
                sessions
                    .get(session_id)
                    .is_some_and(|session| session.is_terminal())
            });
            if terminal {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "permission revocation did not reach terminal state"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            *stop_rx.borrow_and_update(),
            "permission revocation must publish the preview stop signal before dropping ownership"
        );
        assert!(plugin.target_monitor_desired_sessions_for_test().is_empty());
        plugin.session_store().with_sessions(|sessions| {
            let session = sessions.get(session_id).expect("revoked session exists");
            assert_eq!(session.end_reason(), Some(REASON_TARGET_PERMISSION_REVOKED));
            assert!(!session.preview_attached());
        });
        let snapshot = recovery
            .load(session_id)
            .expect("recovery snapshot loads")
            .expect("permission terminal snapshot persists");
        assert_eq!(snapshot.lifecycle_state(), "closed");
        assert_eq!(
            snapshot.terminal_receipt().expect("terminal receipt")["reason_code"],
            serde_json::json!(REASON_TARGET_PERMISSION_REVOKED)
        );
    }

    #[test]
    fn permission_revocation_checkpoint_failure_keeps_transport_and_tracking_owner() {
        let temp = tempfile::tempdir().expect("tempdir");
        let recovery = Arc::new(RemoteDesktopRecoveryStore::new(temp.path().to_path_buf()));
        let sampler = Arc::new(CountingPermissionRevokedTargetSampler::default());
        let monitor = Arc::new(RemoteDesktopTargetMonitor::with_sampler_for_test(
            sampler.clone(),
            Duration::from_millis(250),
        ));
        let plugin = RemoteDesktopPlugin::with_target_monitor_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
            Arc::clone(&recovery),
            monitor,
        );
        let session_id = "rd-monitor-permission-checkpoint-failure";
        let mut session = RemoteDesktopSession::new(test_session_init(
            session_id,
            "easynet:///r/acme/resource/display.monitor-permission-checkpoint-failure",
            vec!["invoke_bidi".into()],
        ));
        session.record_target_observation(TargetObservation::PermissionVerificationRequired {
            detail: "first permission denial".to_string(),
            observed_at_ms: now_ms(),
        });
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let preview_epoch = session
            .attach_preview_transport(stop_tx.clone())
            .expect("preview attaches to verification-pending session")
            .0;
        plugin.session_store().with_sessions(|rows| {
            rows.insert(session_id.to_string(), session);
        });
        let (preview_done_tx, preview_done_rx) = mpsc::channel();
        plugin.transport_manager().activate_preview(
            session_id.to_string(),
            preview_epoch,
            stop_tx,
            preview_done_rx,
        );
        recovery.set_fail_saves_for_test(true);
        RemoteDesktopPlugin::track_target_for_test(&plugin, session_id)
            .expect("target tracking starts the supervisor");

        let sampled_deadline = Instant::now() + Duration::from_secs(2);
        while sampler.calls.load(Ordering::SeqCst) == 0 && Instant::now() < sampled_deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            sampler.calls.load(Ordering::SeqCst) > 0,
            "revocation observation must be attempted"
        );
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !*stop_rx.borrow_and_update(),
            "preview stop must not be sent"
        );
        plugin.session_store().with_sessions(|rows| {
            let session = rows.get(session_id).expect("session remains present");
            assert!(!session.is_terminating());
            assert!(!session.is_terminal());
            assert!(session.preview_attached());
        });
        assert!(plugin
            .target_monitor_desired_sessions_for_test()
            .iter()
            .any(|tracked| tracked == session_id));

        recovery.set_fail_saves_for_test(false);
        plugin.cancel_session_target_tracking(session_id);
        let stop_tx = plugin.session_store().with_sessions(|rows| {
            rows.get_mut(session_id)
                .and_then(|session| session.detach_preview_transport())
        });
        if let Some(stop_tx) = stop_tx {
            let _ = stop_tx.send(true);
        }
        preview_done_tx
            .send(PreviewTaskGroupCompletion)
            .expect("preview worker completion sends");
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
        matches_expected_state: impl Fn(&RemoteDesktopRecoverySnapshot) -> bool,
    ) -> RemoteDesktopRecoverySnapshot {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(snapshot) = recovery.load(session_id).expect("load recovery snapshot") {
                if matches_expected_state(&snapshot) {
                    return snapshot;
                }
            }
            assert!(
                Instant::now() < deadline,
                "target monitor did not persist the expected recovery state"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
