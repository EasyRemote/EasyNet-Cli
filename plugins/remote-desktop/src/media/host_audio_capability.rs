// EasyNet CLI — RemoteApp host-audio runtime capability monitor
// ==============================================================
//
// File: plugins/remote-desktop/src/media/host_audio_capability.rs
// Description: Owns the non-blocking runtime capability snapshot consumed by
// RemoteApp audio offer admission and public session projections.
//
// Protocol Responsibility:
// - None. This is device-local runtime capability state. Negotiated media
//   scope, RTP observations and browser decode evidence remain session facts.
//
// Implementation Approach:
// - Keep one monitor per RemoteDesktopPlugin, never process-global state.
// - Probe native services on a dedicated lifecycle worker and publish each
//   generation atomically with its coordinator state behind one short-held lock.
// - Production OS calls execute in a one-shot sibling media-probe process. A
//   response is committed only after exact validation and successful child
//   exit; timeout/protocol failure kills and reaps the child.
// - Separate compiled support, runtime reachability, system-loopback source
//   readiness and process-tree source readiness.
//
// Usage Contract:
// - Request/session view paths only clone `snapshot()` and never perform OS
//   audio I/O while holding a session-store lock.
// - A snapshot authorizes only whether the browser may offer an audio track.
//   Capture start, sender readiness, packets and browser decode are later,
//   independently evidenced states.
// - Target/PID admission is not cached here; HostAudioSourcePlan owns it.
//
// Architectural Position:
// - RemoteDesktop plugin runtime infrastructure below session orchestration
//   and above the plugin-private media-probe process boundary.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use easynet_remoteapp_native_protocol::media_capabilities::{
    HostAudioCapability as NativeHostAudioCapability, Request as MediaCapabilityRequest,
    Response as MediaCapabilityResponse, SourceReadiness as NativeSourceReadiness,
};

use crate::daemon::plugins::remote_desktop::native_host_process::execute_one_shot_native_host;

// The helper is deliberately one-shot so a native API hang is always
// killable. Refreshes are also triggered by stale reads and capture failures;
// the periodic cadence is therefore a safety refresh, not a polling loop.
const HOST_AUDIO_PROBE_INTERVAL: Duration = Duration::from_secs(30);
const HOST_AUDIO_INITIAL_PROBE_DEADLINE: Duration = Duration::from_millis(2_500);
const HOST_AUDIO_PROBE_ATTEMPT_DEADLINE: Duration = Duration::from_millis(2_500);
const HOST_AUDIO_SNAPSHOT_TTL: Duration = Duration::from_secs(35);
const HOST_AUDIO_FAILURE_BACKOFF: Duration = Duration::from_secs(5);
const HOST_AUDIO_SUPERVISOR_SHUTDOWN_DEADLINE: Duration = Duration::from_millis(500);

pub(in crate::daemon::plugins::remote_desktop) const REASON_HOST_AUDIO_PROBE_PENDING: &str =
    "host_audio_runtime_probe_pending";
#[cfg(any(
    test,
    not(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "windows")
    ))
))]
pub(in crate::daemon::plugins::remote_desktop) const REASON_HOST_AUDIO_SNAPSHOT_EXPIRED: &str =
    "host_audio_runtime_snapshot_expired";
pub(in crate::daemon::plugins::remote_desktop) const REASON_HOST_AUDIO_PROBE_TIMEOUT: &str =
    "host_audio_runtime_probe_timed_out";
pub(in crate::daemon::plugins::remote_desktop) const REASON_HOST_AUDIO_PROBE_PANICKED: &str =
    "host_audio_runtime_probe_panicked";
pub(in crate::daemon::plugins::remote_desktop) const REASON_HOST_AUDIO_SUPERVISOR_UNAVAILABLE:
    &str = "host_audio_runtime_supervisor_unavailable";
pub(in crate::daemon::plugins::remote_desktop) const REASON_HOST_AUDIO_PROBE_SPAWN_FAILED: &str =
    "host_audio_runtime_probe_spawn_failed";
#[cfg(any(test, not(target_os = "macos")))]
pub(in crate::daemon::plugins::remote_desktop) const REASON_ACTIVE_MEDIA_SESSION_AUDIO_UNAVAILABLE:
    &str = "active_media_session_audio_unavailable";
const REASON_HOST_AUDIO_SUPERVISOR_SHUTDOWN_TIMEOUT: &str =
    "host_audio_runtime_supervisor_shutdown_timed_out";
const REASON_HOST_AUDIO_NATIVE_PROCESS_FAILED: &str = "host_audio_native_process_failed";
#[cfg(test)]
pub(in crate::daemon::plugins::remote_desktop) const REASON_PIPEWIRE_RUNTIME_UNAVAILABLE: &str =
    "pipewire_runtime_unavailable";
#[cfg(test)]
pub(in crate::daemon::plugins::remote_desktop) const REASON_PIPEWIRE_DEFAULT_SINK_UNAVAILABLE:
    &str = "pipewire_default_output_sink_unavailable";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum HostAudioSourceClass {
    SystemLoopback,
    ProcessTreeLoopback,
}

impl HostAudioSourceClass {
    pub(in crate::daemon::plugins::remote_desktop) const fn for_target_kind(
        target_kind: crate::daemon::plugins::remote_desktop::target::RemoteDesktopTargetKind,
    ) -> Self {
        match target_kind {
            crate::daemon::plugins::remote_desktop::target::RemoteDesktopTargetKind::Display => {
                Self::SystemLoopback
            }
            crate::daemon::plugins::remote_desktop::target::RemoteDesktopTargetKind::Window
            | crate::daemon::plugins::remote_desktop::target::RemoteDesktopTargetKind::Application => {
                Self::ProcessTreeLoopback
            }
        }
    }

    #[cfg(any(test, not(all(feature = "native-media", target_os = "linux"))))]
    pub(in crate::daemon::plugins::remote_desktop) const fn as_str(self) -> &'static str {
        match self {
            Self::SystemLoopback => "system_loopback",
            Self::ProcessTreeLoopback => "process_tree_loopback",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::SystemLoopback => 0,
            Self::ProcessTreeLoopback => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct HostAudioSourceReadiness {
    ready: bool,
    blocker: Option<String>,
}

impl HostAudioSourceReadiness {
    fn ready() -> Self {
        Self {
            ready: true,
            blocker: None,
        }
    }

    fn blocked(reason: impl Into<String>) -> Self {
        Self {
            ready: false,
            blocker: Some(reason.into()),
        }
    }

    // The readiness/admission accessors below serve the host-audio offer
    // paths, which Linux/Windows native builds route through the media host
    // instead.
    #[cfg(any(
        test,
        not(all(
            feature = "native-media",
            any(target_os = "linux", target_os = "windows")
        ))
    ))]
    pub(in crate::daemon::plugins::remote_desktop) const fn is_ready(&self) -> bool {
        self.ready
    }

    #[cfg(any(
        test,
        not(all(
            feature = "native-media",
            any(target_os = "linux", target_os = "windows")
        ))
    ))]
    pub(in crate::daemon::plugins::remote_desktop) fn blocker(&self) -> Option<&str> {
        self.blocker.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct HostAudioRuntimeSnapshot {
    generation: u64,
    observed_at_ms: u64,
    expires_at_ms: u64,
    expires_at_monotonic: Instant,
    compiled_supported: bool,
    runtime_reachable: bool,
    runtime_blocker: Option<String>,
    system_loopback: HostAudioSourceReadiness,
    process_tree_loopback: HostAudioSourceReadiness,
    diagnostic_detail: Option<String>,
}

impl HostAudioRuntimeSnapshot {
    fn pending() -> Self {
        let observed_at_ms = unix_now_ms();
        let now = Instant::now();
        Self {
            generation: 0,
            observed_at_ms,
            expires_at_ms: observed_at_ms,
            expires_at_monotonic: now,
            compiled_supported: platform_host_audio_compiled(),
            runtime_reachable: false,
            runtime_blocker: Some(REASON_HOST_AUDIO_PROBE_PENDING.to_string()),
            system_loopback: HostAudioSourceReadiness::blocked(REASON_HOST_AUDIO_PROBE_PENDING),
            process_tree_loopback: HostAudioSourceReadiness::blocked(
                REASON_HOST_AUDIO_PROBE_PENDING,
            ),
            diagnostic_detail: None,
        }
    }

    fn observed(
        compiled_supported: bool,
        runtime_reachable: bool,
        runtime_blocker: Option<impl Into<String>>,
        system_loopback: HostAudioSourceReadiness,
        process_tree_loopback: HostAudioSourceReadiness,
        diagnostic_detail: Option<String>,
    ) -> Self {
        let observed_at_ms = unix_now_ms();
        let now = Instant::now();
        Self {
            generation: 0,
            observed_at_ms,
            expires_at_ms: observed_at_ms
                .saturating_add(HOST_AUDIO_SNAPSHOT_TTL.as_millis() as u64),
            expires_at_monotonic: now + HOST_AUDIO_SNAPSHOT_TTL,
            compiled_supported,
            runtime_reachable,
            runtime_blocker: runtime_blocker.map(Into::into),
            system_loopback,
            process_tree_loopback,
            diagnostic_detail,
        }
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn for_test(
        compiled_supported: bool,
        runtime_reachable: bool,
        system_loopback_ready: bool,
        process_tree_loopback_ready: bool,
        blocker: Option<&str>,
    ) -> Self {
        Self::observed(
            compiled_supported,
            runtime_reachable,
            blocker,
            if system_loopback_ready {
                HostAudioSourceReadiness::ready()
            } else {
                HostAudioSourceReadiness::blocked(blocker.unwrap_or("system_loopback_unavailable"))
            },
            if process_tree_loopback_ready {
                HostAudioSourceReadiness::ready()
            } else {
                HostAudioSourceReadiness::blocked(
                    blocker.unwrap_or("process_tree_loopback_unavailable"),
                )
            },
            None,
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn observed_at_ms(&self) -> u64 {
        self.observed_at_ms
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    #[cfg(any(
        test,
        not(all(
            feature = "native-media",
            any(target_os = "linux", target_os = "windows")
        ))
    ))]
    pub(in crate::daemon::plugins::remote_desktop) const fn compiled_supported(&self) -> bool {
        self.compiled_supported
    }

    #[cfg(any(
        test,
        not(all(
            feature = "native-media",
            any(target_os = "linux", target_os = "windows")
        ))
    ))]
    pub(in crate::daemon::plugins::remote_desktop) const fn runtime_reachable(&self) -> bool {
        self.runtime_reachable
    }

    pub(in crate::daemon::plugins::remote_desktop) fn is_fresh(&self) -> bool {
        Instant::now() < self.expires_at_monotonic
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn expire_for_test(&mut self) {
        self.expires_at_monotonic = Instant::now();
        self.expires_at_ms = self.observed_at_ms;
    }

    #[cfg(any(
        test,
        not(all(
            feature = "native-media",
            any(target_os = "linux", target_os = "windows")
        ))
    ))]
    pub(in crate::daemon::plugins::remote_desktop) fn runtime_blocker(&self) -> Option<&str> {
        self.runtime_blocker.as_deref()
    }

    #[cfg(any(
        test,
        not(all(
            feature = "native-media",
            any(target_os = "linux", target_os = "windows")
        ))
    ))]
    pub(in crate::daemon::plugins::remote_desktop) fn source(
        &self,
        source: HostAudioSourceClass,
    ) -> &HostAudioSourceReadiness {
        match source {
            HostAudioSourceClass::SystemLoopback => &self.system_loopback,
            HostAudioSourceClass::ProcessTreeLoopback => &self.process_tree_loopback,
        }
    }

    #[cfg(any(
        test,
        not(all(
            feature = "native-media",
            any(target_os = "linux", target_os = "windows")
        ))
    ))]
    pub(in crate::daemon::plugins::remote_desktop) fn admission_blocker(
        &self,
        source: HostAudioSourceClass,
    ) -> Option<&str> {
        if !self.is_fresh() {
            return Some(REASON_HOST_AUDIO_SNAPSHOT_EXPIRED);
        }
        self.runtime_blocker()
            .or_else(|| self.source(source).blocker())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn diagnostic_detail(&self) -> Option<&str> {
        self.diagnostic_detail.as_deref()
    }

    fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }
}

trait HostAudioRuntimeBackendProbe: Send + Sync {
    fn probe(&self) -> HostAudioRuntimeSnapshot;
}

struct PlatformHostAudioRuntimeBackendProbe {
    process_generation: AtomicU64,
}

impl PlatformHostAudioRuntimeBackendProbe {
    const fn new() -> Self {
        Self {
            process_generation: AtomicU64::new(0),
        }
    }
}

impl HostAudioRuntimeBackendProbe for PlatformHostAudioRuntimeBackendProbe {
    fn probe(&self) -> HostAudioRuntimeSnapshot {
        let result = self.probe_process(HOST_AUDIO_PROBE_ATTEMPT_DEADLINE);
        match result {
            Ok(snapshot) => snapshot,
            Err(detail) => HostAudioRuntimeSnapshot::observed(
                platform_host_audio_compiled(),
                false,
                Some(REASON_HOST_AUDIO_NATIVE_PROCESS_FAILED),
                HostAudioSourceReadiness::blocked(REASON_HOST_AUDIO_NATIVE_PROCESS_FAILED),
                HostAudioSourceReadiness::blocked(REASON_HOST_AUDIO_NATIVE_PROCESS_FAILED),
                Some(detail),
            ),
        }
    }
}

impl PlatformHostAudioRuntimeBackendProbe {
    fn probe_process(&self, deadline: Duration) -> Result<HostAudioRuntimeSnapshot, String> {
        self.probe_process_with_environment(deadline, &[])
    }

    fn probe_process_with_environment(
        &self,
        deadline: Duration,
        extra_environment: &[(std::ffi::OsString, std::ffi::OsString)],
    ) -> Result<HostAudioRuntimeSnapshot, String> {
        let generation = self
            .process_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| "media-probe process generation exhausted".to_string())?
            .saturating_add(1);
        let request_id = generation;
        let request = MediaCapabilityRequest::probe_capabilities(generation, request_id);
        let response: MediaCapabilityResponse = execute_one_shot_native_host(
            generation,
            super::super::MEDIA_HOST_EXECUTABLE,
            "media-probe-helper",
            extra_environment,
            &request,
            deadline,
        )
        .map_err(|error| format!("media-probe host failed: {error}"))?;
        if !response.matches_request(generation, request_id) {
            return Err("media-probe host returned an invalid response envelope".to_string());
        }
        Ok(host_audio_snapshot_from_native(response.capability))
    }
}

fn host_audio_snapshot_from_native(
    capability: NativeHostAudioCapability,
) -> HostAudioRuntimeSnapshot {
    HostAudioRuntimeSnapshot::observed(
        capability.compiled_supported,
        capability.runtime_reachable,
        capability.runtime_blocker,
        host_audio_source_from_native(capability.system_loopback),
        host_audio_source_from_native(capability.process_tree_loopback),
        capability.diagnostic_detail,
    )
}

fn host_audio_source_from_native(source: NativeSourceReadiness) -> HostAudioSourceReadiness {
    if source.ready {
        HostAudioSourceReadiness::ready()
    } else {
        HostAudioSourceReadiness::blocked(
            source
                .blocker
                .unwrap_or_else(|| REASON_HOST_AUDIO_NATIVE_PROCESS_FAILED.to_string()),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostAudioWorkerLifecycle {
    Running,
    Stopping,
}

#[derive(Debug, Clone)]
struct HostAudioFailureLatch {
    reason: String,
    until: Instant,
}

#[derive(Debug)]
struct HostAudioCoordinatorState {
    lifecycle: HostAudioWorkerLifecycle,
    revision: u64,
    generation: u64,
    refresh_requested: bool,
    attempt_running: bool,
    invalidations: [Option<HostAudioFailureLatch>; 2],
    snapshot: HostAudioRuntimeSnapshot,
}

impl HostAudioCoordinatorState {
    fn new() -> Self {
        Self {
            lifecycle: HostAudioWorkerLifecycle::Running,
            revision: 0,
            generation: 0,
            refresh_requested: true,
            attempt_running: false,
            invalidations: [None, None],
            snapshot: HostAudioRuntimeSnapshot::pending(),
        }
    }

    fn expire_latches(&mut self, now: Instant) {
        for latch in &mut self.invalidations {
            if latch.as_ref().is_some_and(|latch| now >= latch.until) {
                *latch = None;
                self.refresh_requested = true;
            }
        }
    }

    fn has_active_latch(&self, now: Instant) -> bool {
        self.invalidations
            .iter()
            .flatten()
            .any(|latch| now < latch.until)
    }
}

struct HostAudioProbeCoordinator {
    state: Mutex<HostAudioCoordinatorState>,
    wake_tx: SyncSender<()>,
}

impl HostAudioProbeCoordinator {
    fn wake(&self) {
        match self.wake_tx.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) | Err(TrySendError::Disconnected(())) => {}
        }
    }
}

struct HostAudioProbeAttempt {
    started_revision: u64,
    deadline: Instant,
    timed_out: bool,
    result_rx: mpsc::Receiver<Result<HostAudioRuntimeSnapshot, String>>,
}

struct HostAudioSupervisor {
    join: JoinHandle<()>,
    exited_rx: mpsc::Receiver<()>,
}

/// Plugin-owned, fixed-state monitor. Its capacity-one channel is only a wake
/// signal: refresh, invalidation and shutdown semantics live in bounded state.
/// The supervisor never performs OS I/O and normally exits through a bounded
/// acknowledgement/join handshake. A missed shutdown deadline detaches it, and
/// at most one native probe attempt may remain blocked in a native API.
pub(in crate::daemon::plugins::remote_desktop) struct HostAudioRuntimeProbe {
    coordinator: Arc<HostAudioProbeCoordinator>,
    supervisor: Mutex<Option<HostAudioSupervisor>>,
}

impl HostAudioRuntimeProbe {
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self::with_backend(Arc::new(PlatformHostAudioRuntimeBackendProbe::new()))
    }

    fn with_backend(backend: Arc<dyn HostAudioRuntimeBackendProbe>) -> Self {
        Self::with_backend_and_deadline(backend, HOST_AUDIO_PROBE_ATTEMPT_DEADLINE)
    }

    fn with_backend_and_deadline(
        backend: Arc<dyn HostAudioRuntimeBackendProbe>,
        attempt_deadline: Duration,
    ) -> Self {
        let (wake_tx, wake_rx) = mpsc::sync_channel(1);
        let coordinator = Arc::new(HostAudioProbeCoordinator {
            state: Mutex::new(HostAudioCoordinatorState::new()),
            wake_tx,
        });
        let (initial_tx, initial_rx) = mpsc::sync_channel(1);
        let coordinator_for_supervisor = Arc::clone(&coordinator);
        let (exited_tx, exited_rx) = mpsc::sync_channel(1);
        let supervisor = std::thread::Builder::new()
            .name("remoteapp-host-audio-supervisor".to_string())
            .spawn(move || {
                let supervisor_result = catch_unwind(AssertUnwindSafe(|| {
                    run_host_audio_probe_supervisor(
                        backend,
                        Arc::clone(&coordinator_for_supervisor),
                        wake_rx,
                        initial_tx,
                        attempt_deadline,
                    )
                }));
                if supervisor_result.is_err() {
                    let mut state = lock_coordinator_state(&coordinator_for_supervisor);
                    if state.lifecycle == HostAudioWorkerLifecycle::Running {
                        state.revision = state.revision.saturating_add(1);
                        state.generation = state.generation.saturating_add(1);
                        let generation = state.generation;
                        state.snapshot = HostAudioRuntimeSnapshot::observed(
                            platform_host_audio_compiled(),
                            false,
                            Some(REASON_HOST_AUDIO_SUPERVISOR_UNAVAILABLE),
                            HostAudioSourceReadiness::blocked(
                                REASON_HOST_AUDIO_SUPERVISOR_UNAVAILABLE,
                            ),
                            HostAudioSourceReadiness::blocked(
                                REASON_HOST_AUDIO_SUPERVISOR_UNAVAILABLE,
                            ),
                            Some("host-audio supervisor panicked".to_string()),
                        )
                        .with_generation(generation);
                    }
                }
                let _ = exited_tx.try_send(());
            })
            .map(|join| HostAudioSupervisor { join, exited_rx })
            .map_err(|error| error.to_string());
        let supervisor = match supervisor {
            Ok(supervisor) => Some(supervisor),
            Err(detail) => {
                let mut state = lock_coordinator_state(&coordinator);
                state.snapshot = HostAudioRuntimeSnapshot::observed(
                    platform_host_audio_compiled(),
                    false,
                    Some(REASON_HOST_AUDIO_SUPERVISOR_UNAVAILABLE),
                    HostAudioSourceReadiness::blocked(REASON_HOST_AUDIO_SUPERVISOR_UNAVAILABLE),
                    HostAudioSourceReadiness::blocked(REASON_HOST_AUDIO_SUPERVISOR_UNAVAILABLE),
                    Some(detail),
                );
                None
            }
        };
        if supervisor.is_some() {
            let initial_deadline = HOST_AUDIO_INITIAL_PROBE_DEADLINE.min(attempt_deadline);
            let _ = initial_rx.recv_timeout(initial_deadline + Duration::from_millis(100));
        }
        Self {
            coordinator,
            supervisor: Mutex::new(supervisor),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn snapshot(&self) -> HostAudioRuntimeSnapshot {
        lock_coordinator_state(&self.coordinator).snapshot.clone()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn refresh(&self) {
        let mut state = lock_coordinator_state(&self.coordinator);
        if state.lifecycle == HostAudioWorkerLifecycle::Running {
            state.refresh_requested = true;
        }
        drop(state);
        self.coordinator.wake();
    }

    pub(in crate::daemon::plugins::remote_desktop) fn invalidate(
        &self,
        source: HostAudioSourceClass,
        reason: impl Into<String>,
    ) {
        let reason = reason.into();
        let now = Instant::now();
        let until = now + HOST_AUDIO_FAILURE_BACKOFF;
        let mut state = lock_coordinator_state(&self.coordinator);
        if state.lifecycle != HostAudioWorkerLifecycle::Running {
            return;
        }
        state.revision = state.revision.saturating_add(1);
        state.generation = state.generation.saturating_add(1);
        state.refresh_requested = true;
        let slot = &mut state.invalidations[source.index()];
        let until = slot
            .as_ref()
            .map_or(until, |current| current.until.max(until));
        *slot = Some(HostAudioFailureLatch {
            reason: reason.clone(),
            until,
        });
        let mut invalid = state.snapshot.clone();
        invalid.generation = state.generation;
        invalid.observed_at_ms = unix_now_ms();
        invalid.expires_at_ms = invalid
            .observed_at_ms
            .saturating_add(until.saturating_duration_since(now).as_millis() as u64);
        invalid.expires_at_monotonic = until;
        match source {
            HostAudioSourceClass::SystemLoopback => {
                invalid.system_loopback = HostAudioSourceReadiness::blocked(reason)
            }
            HostAudioSourceClass::ProcessTreeLoopback => {
                invalid.process_tree_loopback = HostAudioSourceReadiness::blocked(reason)
            }
        }
        state.snapshot = invalid;
        drop(state);
        self.coordinator.wake();
    }

    fn stop(&self) {
        let mut state = lock_coordinator_state(&self.coordinator);
        if state.lifecycle == HostAudioWorkerLifecycle::Stopping {
            return;
        }
        state.lifecycle = HostAudioWorkerLifecycle::Stopping;
        state.revision = state.revision.saturating_add(1);
        state.refresh_requested = false;
        drop(state);
        self.coordinator.wake();
        let supervisor = match self.supervisor.lock() {
            Ok(mut supervisor) => supervisor.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(supervisor) = supervisor {
            if !join_host_audio_supervisor_bounded(
                supervisor,
                HOST_AUDIO_SUPERVISOR_SHUTDOWN_DEADLINE,
            ) {
                eprintln!("[remote-desktop] kind={REASON_HOST_AUDIO_SUPERVISOR_SHUTDOWN_TIMEOUT}");
            }
        }
    }
}

impl Drop for HostAudioRuntimeProbe {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_host_audio_probe_supervisor(
    backend: Arc<dyn HostAudioRuntimeBackendProbe>,
    coordinator: Arc<HostAudioProbeCoordinator>,
    wake_rx: mpsc::Receiver<()>,
    initial_tx: SyncSender<()>,
    attempt_deadline: Duration,
) {
    let mut attempt: Option<HostAudioProbeAttempt> = None;
    let mut initial_reported = false;
    let mut next_periodic_probe = Instant::now();
    loop {
        let now = Instant::now();
        {
            let mut state = lock_coordinator_state(&coordinator);
            if state.lifecycle == HostAudioWorkerLifecycle::Stopping {
                break;
            }
            state.expire_latches(now);
        }

        if let Some(active) = attempt.as_mut() {
            match active.result_rx.try_recv() {
                Ok(result) => {
                    let mut state = lock_coordinator_state(&coordinator);
                    state.attempt_running = false;
                    if state.lifecycle == HostAudioWorkerLifecycle::Running
                        && !active.timed_out
                        && state.revision == active.started_revision
                    {
                        state.generation = state.generation.saturating_add(1);
                        let generation = state.generation;
                        let observed = match result {
                            Ok(observed) => apply_active_failure_latches(observed, &state, now),
                            Err(detail) => HostAudioRuntimeSnapshot::observed(
                                platform_host_audio_compiled(),
                                false,
                                Some(REASON_HOST_AUDIO_PROBE_PANICKED),
                                HostAudioSourceReadiness::blocked(REASON_HOST_AUDIO_PROBE_PANICKED),
                                HostAudioSourceReadiness::blocked(REASON_HOST_AUDIO_PROBE_PANICKED),
                                Some(detail),
                            ),
                        }
                        .with_generation(generation);
                        state.snapshot = observed;
                    }
                    attempt = None;
                    if !initial_reported {
                        let _ = initial_tx.try_send(());
                        initial_reported = true;
                    }
                    continue;
                }
                Err(TryRecvError::Disconnected) => {
                    let mut state = lock_coordinator_state(&coordinator);
                    state.attempt_running = false;
                    state.refresh_requested = true;
                    attempt = None;
                    continue;
                }
                Err(TryRecvError::Empty) if !active.timed_out && now >= active.deadline => {
                    active.timed_out = true;
                    let mut state = lock_coordinator_state(&coordinator);
                    if state.lifecycle == HostAudioWorkerLifecycle::Running
                        && state.revision == active.started_revision
                    {
                        state.revision = state.revision.saturating_add(1);
                        state.generation = state.generation.saturating_add(1);
                        state.refresh_requested = true;
                        let generation = state.generation;
                        state.snapshot = HostAudioRuntimeSnapshot::observed(
                            platform_host_audio_compiled(),
                            false,
                            Some(REASON_HOST_AUDIO_PROBE_TIMEOUT),
                            HostAudioSourceReadiness::blocked(REASON_HOST_AUDIO_PROBE_TIMEOUT),
                            HostAudioSourceReadiness::blocked(REASON_HOST_AUDIO_PROBE_TIMEOUT),
                            Some("native host-audio probe exceeded its deadline".to_string()),
                        )
                        .with_generation(generation);
                    }
                    if !initial_reported {
                        let _ = initial_tx.try_send(());
                        initial_reported = true;
                    }
                }
                Err(TryRecvError::Empty) => {}
            }
        }

        let now = Instant::now();
        if attempt.is_none() {
            let mut state = lock_coordinator_state(&coordinator);
            state.expire_latches(now);
            if !state.has_active_latch(now)
                && (state.refresh_requested || now >= next_periodic_probe)
            {
                state.refresh_requested = false;
                state.attempt_running = true;
                let started_revision = state.revision;
                drop(state);
                let spawned = spawn_host_audio_probe_attempt(
                    Arc::clone(&backend),
                    Arc::clone(&coordinator),
                    started_revision,
                    attempt_deadline,
                );
                next_periodic_probe = now + HOST_AUDIO_PROBE_INTERVAL;
                match spawned {
                    Ok(spawned) => attempt = Some(spawned),
                    Err(error) => {
                        let mut state = lock_coordinator_state(&coordinator);
                        state.attempt_running = false;
                        state.generation = state.generation.saturating_add(1);
                        let generation = state.generation;
                        state.snapshot = HostAudioRuntimeSnapshot::observed(
                            platform_host_audio_compiled(),
                            false,
                            Some(REASON_HOST_AUDIO_PROBE_SPAWN_FAILED),
                            HostAudioSourceReadiness::blocked(REASON_HOST_AUDIO_PROBE_SPAWN_FAILED),
                            HostAudioSourceReadiness::blocked(REASON_HOST_AUDIO_PROBE_SPAWN_FAILED),
                            Some(error.to_string()),
                        )
                        .with_generation(generation);
                        if !initial_reported {
                            let _ = initial_tx.try_send(());
                            initial_reported = true;
                        }
                    }
                }
                continue;
            }
        }

        let now = Instant::now();
        let wait = match attempt.as_ref() {
            Some(attempt) if !attempt.timed_out => attempt.deadline.saturating_duration_since(now),
            Some(_) => HOST_AUDIO_PROBE_INTERVAL,
            None => {
                let state = lock_coordinator_state(&coordinator);
                state
                    .invalidations
                    .iter()
                    .flatten()
                    .filter(|latch| latch.until > now)
                    .map(|latch| latch.until.saturating_duration_since(now))
                    .min()
                    .unwrap_or_else(|| next_periodic_probe.saturating_duration_since(now))
            }
        }
        .min(HOST_AUDIO_PROBE_INTERVAL);
        match wake_rx.recv_timeout(wait) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn spawn_host_audio_probe_attempt(
    backend: Arc<dyn HostAudioRuntimeBackendProbe>,
    coordinator: Arc<HostAudioProbeCoordinator>,
    started_revision: u64,
    attempt_deadline: Duration,
) -> std::io::Result<HostAudioProbeAttempt> {
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("remoteapp-host-audio-probe-attempt".to_string())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| backend.probe()))
                .map_err(|_| "native host-audio probe panicked".to_string());
            let _ = result_tx.try_send(result);
            coordinator.wake();
        })?;
    Ok(HostAudioProbeAttempt {
        started_revision,
        deadline: Instant::now() + attempt_deadline,
        timed_out: false,
        result_rx,
    })
}

fn apply_active_failure_latches(
    mut observed: HostAudioRuntimeSnapshot,
    state: &HostAudioCoordinatorState,
    now: Instant,
) -> HostAudioRuntimeSnapshot {
    for (index, latch) in state.invalidations.iter().enumerate() {
        let Some(latch) = latch.as_ref().filter(|latch| now < latch.until) else {
            continue;
        };
        let blocked = HostAudioSourceReadiness::blocked(latch.reason.clone());
        if index == HostAudioSourceClass::SystemLoopback.index() {
            observed.system_loopback = blocked;
        } else {
            observed.process_tree_loopback = blocked;
        }
    }
    observed
}

fn lock_coordinator_state(
    coordinator: &HostAudioProbeCoordinator,
) -> std::sync::MutexGuard<'_, HostAudioCoordinatorState> {
    match coordinator.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn join_host_audio_supervisor_bounded(
    supervisor: HostAudioSupervisor,
    shutdown_deadline: Duration,
) -> bool {
    let HostAudioSupervisor { join, exited_rx } = supervisor;
    let deadline = Instant::now() + shutdown_deadline;
    let _ = exited_rx.recv_timeout(shutdown_deadline);
    while !join.is_finished() && Instant::now() < deadline {
        std::thread::park_timeout(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(1)),
        );
    }
    if !join.is_finished() {
        drop(join);
        return false;
    }
    let _ = join.join();
    true
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

const fn platform_host_audio_compiled() -> bool {
    cfg!(all(feature = "native-media", target_os = "macos"))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Condvar;

    use super::*;

    struct SequencedProbe {
        calls: AtomicUsize,
        snapshots: Mutex<VecDeque<HostAudioRuntimeSnapshot>>,
    }

    impl HostAudioRuntimeBackendProbe for SequencedProbe {
        fn probe(&self) -> HostAudioRuntimeSnapshot {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.snapshots
                .lock()
                .expect("sequenced probe snapshots")
                .pop_front()
                .unwrap_or_else(HostAudioRuntimeSnapshot::pending)
        }
    }

    struct BlockingSecondProbe {
        calls: AtomicUsize,
        gate: Arc<(Mutex<bool>, Condvar)>,
    }

    impl HostAudioRuntimeBackendProbe for BlockingSecondProbe {
        fn probe(&self) -> HostAudioRuntimeSnapshot {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == 2 {
                let (released, signal) = &*self.gate;
                let mut released = released.lock().expect("blocking probe gate");
                while !*released {
                    released = signal.wait(released).expect("blocking probe wait");
                }
            }
            HostAudioRuntimeSnapshot::for_test(true, true, true, true, None)
        }
    }

    struct AlwaysBlockingProbe {
        calls: AtomicUsize,
        gate: Arc<(Mutex<bool>, Condvar)>,
    }

    impl HostAudioRuntimeBackendProbe for AlwaysBlockingProbe {
        fn probe(&self) -> HostAudioRuntimeSnapshot {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let (released, signal) = &*self.gate;
            let mut released = released.lock().expect("blocking probe gate");
            while !*released {
                released = signal.wait(released).expect("blocking probe wait");
            }
            HostAudioRuntimeSnapshot::for_test(true, true, true, true, None)
        }
    }

    fn release(gate: &Arc<(Mutex<bool>, Condvar)>) {
        let (released, signal) = &**gate;
        *released.lock().expect("blocking probe gate") = true;
        signal.notify_all();
    }

    fn wait_for_calls(calls: &AtomicUsize, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while calls.load(Ordering::SeqCst) < expected && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(calls.load(Ordering::SeqCst), expected);
    }

    #[test]
    fn snapshot_keeps_compiled_runtime_and_source_readiness_distinct() {
        let snapshot = HostAudioRuntimeSnapshot::for_test(
            true,
            true,
            false,
            true,
            Some(REASON_PIPEWIRE_DEFAULT_SINK_UNAVAILABLE),
        );
        assert!(snapshot.compiled_supported());
        assert!(snapshot.runtime_reachable());
        assert!(!snapshot
            .source(HostAudioSourceClass::SystemLoopback)
            .is_ready());
        assert!(snapshot
            .source(HostAudioSourceClass::ProcessTreeLoopback)
            .is_ready());
    }

    #[test]
    fn snapshot_ttl_covers_periodic_probe_and_full_attempt_deadline() {
        assert!(
            HOST_AUDIO_SNAPSHOT_TTL > HOST_AUDIO_PROBE_INTERVAL + HOST_AUDIO_PROBE_ATTEMPT_DEADLINE,
            "a successful observation must remain fresh until the next periodic attempt settles"
        );
    }

    #[test]
    fn real_media_probe_process_round_trips_and_exits_before_commit() {
        crate::daemon::plugins::remote_desktop::native_host_process::sibling_executable(
            super::super::super::MEDIA_HOST_EXECUTABLE,
        )
        .expect(
            "build the sibling media-probe host with \
             `cargo build -p easynet-remoteapp-media-host`",
        );
        let backend = PlatformHostAudioRuntimeBackendProbe::new();
        let snapshot = backend
            .probe_process(Duration::from_secs(3))
            .expect("real media-probe process returns and exits successfully");
        assert!(snapshot.is_fresh());
        assert_ne!(
            snapshot.runtime_blocker(),
            Some(REASON_HOST_AUDIO_NATIVE_PROCESS_FAILED)
        );
    }

    #[cfg(feature = "remoteapp-e2e-fault-injection")]
    #[test]
    fn hung_media_probe_process_is_killed_reaped_and_replaced() {
        let backend = PlatformHostAudioRuntimeBackendProbe::new();
        let started = Instant::now();
        let failure = backend
            .probe_process_with_environment(
                Duration::from_millis(100),
                &[(
                    "EASYNET_REMOTEAPP_MEDIA_PROBE_TEST_FAULT".into(),
                    "hang".into(),
                )],
            )
            .expect_err("fault-injected media probe must time out");
        assert!(failure.contains("await media-probe response"));
        assert!(started.elapsed() < Duration::from_millis(500));
        backend
            .probe_process(Duration::from_secs(3))
            .expect("replacement media-probe process must recover");
    }

    #[test]
    fn native_probe_projection_keeps_source_readiness_independent() {
        let process_ready = host_audio_snapshot_from_native(NativeHostAudioCapability::new(
            true,
            true,
            None::<String>,
            NativeSourceReadiness::blocked("render_endpoint_unavailable"),
            NativeSourceReadiness::ready(),
            Some("render endpoint enumeration failed".to_string()),
        ));
        assert!(process_ready.runtime_reachable());
        assert!(!process_ready
            .source(HostAudioSourceClass::SystemLoopback)
            .is_ready());
        assert!(process_ready
            .source(HostAudioSourceClass::ProcessTreeLoopback)
            .is_ready());

        let system_ready = host_audio_snapshot_from_native(NativeHostAudioCapability::new(
            true,
            true,
            None::<String>,
            NativeSourceReadiness::ready(),
            NativeSourceReadiness::blocked("process_loopback_unavailable"),
            Some("process activation failed".to_string()),
        ));
        assert!(system_ready.runtime_reachable());
        assert!(system_ready
            .source(HostAudioSourceClass::SystemLoopback)
            .is_ready());
        assert!(!system_ready
            .source(HostAudioSourceClass::ProcessTreeLoopback)
            .is_ready());
    }

    #[test]
    fn plugin_owned_worker_prewarms_and_refreshes_one_generation_at_a_time() {
        let backend = Arc::new(SequencedProbe {
            calls: AtomicUsize::new(0),
            snapshots: Mutex::new(VecDeque::from([
                HostAudioRuntimeSnapshot::for_test(
                    true,
                    false,
                    false,
                    false,
                    Some(REASON_PIPEWIRE_RUNTIME_UNAVAILABLE),
                ),
                HostAudioRuntimeSnapshot::for_test(true, true, true, true, None),
            ])),
        });
        let probe = HostAudioRuntimeProbe::with_backend(backend.clone());
        let initial = probe.snapshot();
        assert_eq!(initial.generation(), 1);
        assert!(!initial.runtime_reachable());

        probe.refresh();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while probe.snapshot().generation() < 2 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        let refreshed = probe.snapshot();
        assert_eq!(refreshed.generation(), 2);
        assert!(refreshed.runtime_reachable());
        assert_eq!(backend.calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn invalidate_is_synchronous_source_scoped_and_obsoletes_inflight_success() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let backend = Arc::new(BlockingSecondProbe {
            calls: AtomicUsize::new(0),
            gate: Arc::clone(&gate),
        });
        let probe = HostAudioRuntimeProbe::with_backend(backend.clone());
        assert!(probe.snapshot().runtime_reachable());

        probe.refresh();
        wait_for_calls(&backend.calls, 2);
        for _ in 0..10_000 {
            probe.refresh();
        }
        {
            let state = lock_coordinator_state(&probe.coordinator);
            assert!(state.attempt_running);
            assert!(state.refresh_requested, "refresh storm is one pending bit");
        }

        probe.invalidate(
            HostAudioSourceClass::ProcessTreeLoopback,
            "process_audio_setup_failed",
        );
        let invalid = probe.snapshot();
        assert!(invalid
            .source(HostAudioSourceClass::SystemLoopback)
            .is_ready());
        assert_eq!(
            invalid
                .source(HostAudioSourceClass::ProcessTreeLoopback)
                .blocker(),
            Some("process_audio_setup_failed")
        );

        release(&gate);
        let deadline = Instant::now() + Duration::from_secs(1);
        while lock_coordinator_state(&probe.coordinator).attempt_running
            && Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        assert_eq!(
            probe
                .snapshot()
                .source(HostAudioSourceClass::ProcessTreeLoopback)
                .blocker(),
            Some("process_audio_setup_failed"),
            "late success from the invalidated revision must not resurrect readiness"
        );
        assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn obsolete_attempt_timeout_does_not_override_newer_source_invalidation() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let backend = Arc::new(BlockingSecondProbe {
            calls: AtomicUsize::new(0),
            gate: Arc::clone(&gate),
        });
        let probe = HostAudioRuntimeProbe::with_backend_and_deadline(
            backend.clone(),
            Duration::from_millis(10),
        );
        assert!(probe.snapshot().runtime_reachable());

        probe.refresh();
        wait_for_calls(&backend.calls, 2);
        probe.invalidate(
            HostAudioSourceClass::ProcessTreeLoopback,
            "process_audio_setup_failed",
        );
        let (revision_after_invalidation, generation_after_invalidation) = {
            let state = lock_coordinator_state(&probe.coordinator);
            (state.revision, state.generation)
        };

        let timeout_observation_deadline = Instant::now() + Duration::from_millis(100);
        while Instant::now() < timeout_observation_deadline {
            std::thread::park_timeout(Duration::from_millis(1));
        }
        let (revision, generation, snapshot) = {
            let state = lock_coordinator_state(&probe.coordinator);
            (state.revision, state.generation, state.snapshot.clone())
        };
        release(&gate);

        assert_eq!(revision, revision_after_invalidation);
        assert_eq!(generation, generation_after_invalidation);
        assert!(snapshot
            .source(HostAudioSourceClass::SystemLoopback)
            .is_ready());
        assert_eq!(
            snapshot
                .source(HostAudioSourceClass::ProcessTreeLoopback)
                .blocker(),
            Some("process_audio_setup_failed")
        );
    }

    #[test]
    fn supervisor_join_has_a_hard_deadline_when_exit_ack_never_arrives() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let gate_for_supervisor = Arc::clone(&gate);
        let (exited_tx, exited_rx) = mpsc::sync_channel(1);
        let (completed_tx, completed_rx) = mpsc::sync_channel(1);
        let join = std::thread::spawn(move || {
            let (released, signal) = &*gate_for_supervisor;
            let mut released = released.lock().expect("supervisor stall gate");
            while !*released {
                released = signal.wait(released).expect("supervisor stall wait");
            }
            let _ = exited_tx.try_send(());
            let _ = completed_tx.try_send(());
        });

        let started = Instant::now();
        let joined = join_host_audio_supervisor_bounded(
            HostAudioSupervisor { join, exited_rx },
            Duration::from_millis(25),
        );
        let elapsed = started.elapsed();
        release(&gate);
        completed_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("detached supervisor exits after its test stall is released");

        assert!(!joined, "a supervisor without an exit ack must be detached");
        assert!(
            elapsed < Duration::from_millis(250),
            "supervisor shutdown must honor its hard deadline"
        );
    }

    #[test]
    fn blocked_native_probe_does_not_block_supervisor_shutdown() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let backend = Arc::new(AlwaysBlockingProbe {
            calls: AtomicUsize::new(0),
            gate: Arc::clone(&gate),
        });
        let probe = HostAudioRuntimeProbe::with_backend_and_deadline(
            backend.clone(),
            Duration::from_millis(25),
        );
        wait_for_calls(&backend.calls, 1);
        assert_eq!(
            probe.snapshot().runtime_blocker(),
            Some(REASON_HOST_AUDIO_PROBE_TIMEOUT)
        );

        let started = Instant::now();
        drop(probe);
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "daemon shutdown must join only the non-blocking supervisor"
        );
        assert_eq!(
            backend.calls.load(Ordering::SeqCst),
            1,
            "a timed-out attempt must not spawn an unbounded replacement thread"
        );
        release(&gate);
    }
}
