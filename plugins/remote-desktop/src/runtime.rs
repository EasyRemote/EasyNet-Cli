// EasyNet CLI — remote desktop plugin runtime
// ==============================================
//
// File: plugins/remote-desktop/src/runtime.rs
// Description: Device-side remote desktop control-plane handlers.
//
// Protocol Responsibility:
// - Implements the daemon-owned remote desktop session contract.
// - Requires the acted-on display/window/application resource to be
//   supplied as Envelope.subject, never as JSON arguments.
// - Keeps production media transport explicit: this module creates and
//   tracks sessions, but does not pretend the preview JPEG stream is a
//   WebRTC remote desktop media plane.
//
// Implementation Approach:
// - In-process session store mirrors the existing voice-call v1 model:
//   deterministic state transitions, bounded event snapshots, and
//   idempotent terminal close.
// - `preview_stream` and InvokeBidi are surfaced as diagnostic transports.
//   They never mark the production WebRTC media plane ready.
//
// Usage Contract:
// - `remote_desktop.create_session` MUST be called with
//   `subject = resource_ura` for a display/window/application.
// - WebRTC SDP/ICE calls are accepted, audited, and routed to a device-side
//   WebRTC endpoint when the local media SDK exposes a transport-ready backend.
//
// Architectural Position:
// - CLI device adapter layer. Axon owns generic invocation semantics; this
//   file owns remote desktop runtime behavior and product state transitions.

use std::sync::Arc;

use crate::daemon::ability::builtins::resources::media::screen_snapshot::ScreenSnapshotBackend;
use crate::daemon::plugins::remote_desktop::config::RemoteDesktopRuntimeConfig;
use crate::daemon::plugins::remote_desktop::consent_registry::RemoteDesktopConsentRegistry;
use crate::daemon::plugins::remote_desktop::lease_monitor::RemoteDesktopLeaseMonitor;
use crate::daemon::plugins::remote_desktop::media::host_audio_capability::{
    HostAudioRuntimeProbe, HostAudioRuntimeSnapshot, HostAudioSourceClass,
};
#[cfg(test)]
use crate::daemon::plugins::remote_desktop::relay_lease::UnavailableRemoteDesktopRelayLeaseProvider;
use crate::daemon::plugins::remote_desktop::relay_lease::{
    RemoteDesktopRelayLeaseAvailability, RemoteDesktopRelayLeaseProvider,
};
use crate::daemon::plugins::remote_desktop::session::{
    now_ms, RemoteDesktopRelayLeaseRotation, RemoteDesktopSession,
};
use crate::daemon::plugins::remote_desktop::session_creation::{
    PlatformRemoteAppTargetBindingVerifier, RemoteAppTargetBindingVerifier,
};
use crate::daemon::plugins::remote_desktop::session_recovery::{
    RemoteDesktopRecoverySnapshot, RemoteDesktopRecoveryStore,
};
use crate::daemon::plugins::remote_desktop::session_store::max_session_rows_for_active_limit;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::target_focus::{
    PlatformRemoteAppTargetFocusController, RemoteAppTargetFocusController,
};
use crate::daemon::plugins::remote_desktop::target_monitor::RemoteDesktopTargetMonitor;
use crate::daemon::plugins::remote_desktop::target_snapshot::TargetSnapshotDeadlineExecutor;
use crate::daemon::plugins::remote_desktop::transport::{
    DirectWebRtcEndpoint, RemoteDesktopTransportManager,
};

/// Runtime-owned state for the remote desktop plugin.
///
/// Invariant 1: every mutable session row is reachable only through this
/// plugin instance, never through process-global storage.
/// Invariant 2: transport handles are torn down through the same plugin
/// instance that created them, so lease expiry, explicit close, and test reset
/// share one lifecycle path.
#[derive(Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopPlugin {
    sessions: Arc<RemoteDesktopSessionStore>,
    consent: Arc<RemoteDesktopConsentRegistry>,
    lease_monitor: Arc<RemoteDesktopLeaseMonitor>,
    target_monitor: Arc<RemoteDesktopTargetMonitor>,
    transports: Arc<RemoteDesktopTransportManager>,
    host_audio_probe: Arc<HostAudioRuntimeProbe>,
    recovery: Arc<RemoteDesktopRecoveryStore>,
    screen_backend: Arc<dyn ScreenSnapshotBackend>,
    target_binding_verifier: Arc<dyn RemoteAppTargetBindingVerifier>,
    target_focus_controller: Arc<dyn RemoteAppTargetFocusController>,
    relay_lease_provider: Arc<dyn RemoteDesktopRelayLeaseProvider>,
    config: RemoteDesktopRuntimeConfig,
}

impl RemoteDesktopPlugin {
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        screen_backend: Arc<dyn ScreenSnapshotBackend>,
        config: RemoteDesktopRuntimeConfig,
        relay_lease_provider: Arc<dyn RemoteDesktopRelayLeaseProvider>,
    ) -> Arc<Self> {
        Self::with_platform_target_focus(
            screen_backend,
            Arc::new(PlatformRemoteAppTargetBindingVerifier),
            config,
            Arc::new(RemoteDesktopRecoveryStore::daemon_default()),
            Arc::new(RemoteDesktopTargetMonitor::new()),
            relay_lease_provider,
            true,
        )
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn with_target_binding_verifier(
        screen_backend: Arc<dyn ScreenSnapshotBackend>,
        target_binding_verifier: Arc<dyn RemoteAppTargetBindingVerifier>,
        config: RemoteDesktopRuntimeConfig,
    ) -> Arc<Self> {
        Self::with_platform_target_focus(
            screen_backend,
            target_binding_verifier,
            config,
            Arc::new(RemoteDesktopRecoveryStore::daemon_default()),
            Arc::new(RemoteDesktopTargetMonitor::stable_for_test()),
            Arc::new(UnavailableRemoteDesktopRelayLeaseProvider),
            false,
        )
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn with_recovery_store_for_test(
        screen_backend: Arc<dyn ScreenSnapshotBackend>,
        target_binding_verifier: Arc<dyn RemoteAppTargetBindingVerifier>,
        config: RemoteDesktopRuntimeConfig,
        recovery: Arc<RemoteDesktopRecoveryStore>,
    ) -> Arc<Self> {
        Self::with_platform_target_focus(
            screen_backend,
            target_binding_verifier,
            config,
            recovery,
            Arc::new(RemoteDesktopTargetMonitor::stable_for_test()),
            Arc::new(UnavailableRemoteDesktopRelayLeaseProvider),
            true,
        )
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn with_target_monitor_for_test(
        screen_backend: Arc<dyn ScreenSnapshotBackend>,
        target_binding_verifier: Arc<dyn RemoteAppTargetBindingVerifier>,
        config: RemoteDesktopRuntimeConfig,
        recovery: Arc<RemoteDesktopRecoveryStore>,
        target_monitor: Arc<RemoteDesktopTargetMonitor>,
    ) -> Arc<Self> {
        Self::with_platform_target_focus(
            screen_backend,
            target_binding_verifier,
            config,
            recovery,
            target_monitor,
            Arc::new(UnavailableRemoteDesktopRelayLeaseProvider),
            true,
        )
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn with_target_focus_controller_for_test(
        screen_backend: Arc<dyn ScreenSnapshotBackend>,
        target_binding_verifier: Arc<dyn RemoteAppTargetBindingVerifier>,
        target_focus_controller: Arc<dyn RemoteAppTargetFocusController>,
        config: RemoteDesktopRuntimeConfig,
    ) -> Arc<Self> {
        Self::with_target_binding_verifier_inner(
            screen_backend,
            target_binding_verifier,
            config,
            Arc::new(RemoteDesktopRecoveryStore::daemon_default()),
            Arc::new(RemoteDesktopTargetMonitor::stable_for_test()),
            target_focus_controller,
            Arc::new(UnavailableRemoteDesktopRelayLeaseProvider),
            false,
        )
    }

    #[cfg(test)]
    pub(in crate::daemon) fn with_relay_lease_provider_for_test(
        screen_backend: Arc<dyn ScreenSnapshotBackend>,
        target_binding_verifier: Arc<dyn RemoteAppTargetBindingVerifier>,
        config: RemoteDesktopRuntimeConfig,
        relay_lease_provider: Arc<dyn RemoteDesktopRelayLeaseProvider>,
    ) -> Arc<Self> {
        Self::with_platform_target_focus(
            screen_backend,
            target_binding_verifier,
            config,
            Arc::new(RemoteDesktopRecoveryStore::daemon_default()),
            Arc::new(RemoteDesktopTargetMonitor::stable_for_test()),
            relay_lease_provider,
            false,
        )
    }

    fn with_target_binding_verifier_inner(
        screen_backend: Arc<dyn ScreenSnapshotBackend>,
        target_binding_verifier: Arc<dyn RemoteAppTargetBindingVerifier>,
        config: RemoteDesktopRuntimeConfig,
        recovery: Arc<RemoteDesktopRecoveryStore>,
        target_monitor: Arc<RemoteDesktopTargetMonitor>,
        target_focus_controller: Arc<dyn RemoteAppTargetFocusController>,
        relay_lease_provider: Arc<dyn RemoteDesktopRelayLeaseProvider>,
        rehydrate: bool,
    ) -> Arc<Self> {
        let plugin = Arc::new(Self {
            sessions: Arc::new(RemoteDesktopSessionStore::new()),
            consent: Arc::new(RemoteDesktopConsentRegistry::new(
                config.max_sessions().saturating_mul(4),
            )),
            lease_monitor: Arc::new(RemoteDesktopLeaseMonitor::new()),
            target_monitor,
            transports: Arc::new(RemoteDesktopTransportManager::new()),
            host_audio_probe: Arc::new(HostAudioRuntimeProbe::new()),
            recovery,
            screen_backend,
            target_binding_verifier,
            target_focus_controller,
            relay_lease_provider,
            config,
        });
        if rehydrate {
            if let Err(err) = Self::rehydrate_recovery_snapshots(&plugin) {
                eprintln!("[remote-desktop] recovery snapshot rehydration failed: {err}");
            }
        }
        plugin
    }

    fn with_platform_target_focus(
        screen_backend: Arc<dyn ScreenSnapshotBackend>,
        target_binding_verifier: Arc<dyn RemoteAppTargetBindingVerifier>,
        config: RemoteDesktopRuntimeConfig,
        recovery: Arc<RemoteDesktopRecoveryStore>,
        target_monitor: Arc<RemoteDesktopTargetMonitor>,
        relay_lease_provider: Arc<dyn RemoteDesktopRelayLeaseProvider>,
        rehydrate: bool,
    ) -> Arc<Self> {
        let target_focus_controller = Arc::new(PlatformRemoteAppTargetFocusController::new(
            target_monitor.snapshot_executor(),
        ));
        Self::with_target_binding_verifier_inner(
            screen_backend,
            target_binding_verifier,
            config,
            recovery,
            target_monitor,
            target_focus_controller,
            relay_lease_provider,
            rehydrate,
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn config(
        &self,
    ) -> RemoteDesktopRuntimeConfig {
        self.config
    }

    pub(in crate::daemon::plugins::remote_desktop) fn session_store(
        &self,
    ) -> Arc<RemoteDesktopSessionStore> {
        Arc::clone(&self.sessions)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_snapshot_executor(
        &self,
    ) -> Arc<TargetSnapshotDeadlineExecutor> {
        self.target_monitor.snapshot_executor()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn consent_registry(
        &self,
    ) -> Arc<RemoteDesktopConsentRegistry> {
        Arc::clone(&self.consent)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_focus_controller(
        &self,
    ) -> Arc<dyn RemoteAppTargetFocusController> {
        Arc::clone(&self.target_focus_controller)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn schedule_session_lease(
        plugin: &Arc<Self>,
        session_id: String,
        lease_expires_at_ms: u64,
    ) -> anyhow::Result<()> {
        plugin
            .lease_monitor
            .schedule(plugin, session_id, lease_expires_at_ms)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn cancel_session_lease(
        &self,
        session_id: &str,
    ) {
        self.lease_monitor.cancel(session_id);
    }

    pub(in crate::daemon) fn schedule_relay_lease_refresh(
        plugin: &Arc<Self>,
        session_id: String,
        refresh_after_ms: u64,
    ) -> anyhow::Result<()> {
        plugin
            .lease_monitor
            .schedule_relay_refresh(plugin, session_id, refresh_after_ms)
    }

    pub(in crate::daemon) fn refresh_relay_lease_from_watchdog(
        plugin: &Arc<Self>,
        session_id: &str,
        expected_refresh_after_ms: u64,
    ) -> Option<u64> {
        let current = plugin.session_store().with_sessions(|sessions| {
            let session = sessions.get(session_id)?;
            let lease = session.active_relay_lease()?;
            (lease.refresh_after_ms() == expected_refresh_after_ms).then(|| lease.clone())
        })?;
        let current_lease_id = current.lease_id().to_string();
        let current_expires_at_ms = current.expires_at_ms();
        let outcome = plugin
            .relay_lease_provider()
            .acquire(current.session_id(), current.resource_ura());
        match outcome {
            Ok(RemoteDesktopRelayLeaseAvailability::Active(refreshed)) => {
                commit_relay_lease_refresh(plugin, session_id, &current_lease_id, refreshed)
            }
            Ok(RemoteDesktopRelayLeaseAvailability::Unavailable { reason }) => {
                retry_relay_refresh_deadline(
                    plugin,
                    session_id,
                    &current_lease_id,
                    current_expires_at_ms,
                    &reason,
                )
            }
            Err(error) => retry_relay_refresh_deadline(
                plugin,
                session_id,
                &current_lease_id,
                current_expires_at_ms,
                &error.to_string(),
            ),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn lease_monitor(
        &self,
    ) -> Arc<RemoteDesktopLeaseMonitor> {
        Arc::clone(&self.lease_monitor)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn track_session_target(
        plugin: &Arc<Self>,
        session_id: String,
    ) -> anyhow::Result<()> {
        plugin.target_monitor.track(plugin, session_id)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn cancel_session_target_tracking(
        &self,
        session_id: &str,
    ) {
        self.target_monitor.cancel(session_id);
    }

    pub(in crate::daemon::plugins::remote_desktop) fn endpoint(
        &self,
        session_id: &str,
    ) -> Option<DirectWebRtcEndpoint> {
        self.transports.endpoint(session_id)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn transport_manager(
        &self,
    ) -> Arc<RemoteDesktopTransportManager> {
        Arc::clone(&self.transports)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn host_audio_runtime_snapshot(
        &self,
    ) -> HostAudioRuntimeSnapshot {
        let snapshot = self.host_audio_probe.snapshot();
        if !snapshot.is_fresh() {
            self.host_audio_probe.refresh();
        }
        snapshot
    }

    pub(in crate::daemon::plugins::remote_desktop) fn session_view(
        &self,
        session: &RemoteDesktopSession,
    ) -> serde_json::Value {
        let audio_runtime = self.host_audio_runtime_snapshot();
        crate::daemon::plugins::remote_desktop::view::serialize_session_with_audio_runtime(
            session,
            &audio_runtime,
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn session_view_with_token(
        &self,
        session: &RemoteDesktopSession,
    ) -> serde_json::Value {
        let audio_runtime = self.host_audio_runtime_snapshot();
        crate::daemon::plugins::remote_desktop::view::serialize_session_with_token_and_audio_runtime(
            session,
            &audio_runtime,
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn invalidate_host_audio_runtime(
        &self,
        source: HostAudioSourceClass,
        reason: impl Into<String>,
    ) {
        self.host_audio_probe.invalidate(source, reason);
    }

    pub(in crate::daemon::plugins::remote_desktop) fn recovery_store(
        &self,
    ) -> Arc<RemoteDesktopRecoveryStore> {
        Arc::clone(&self.recovery)
    }

    pub(in crate::daemon) fn relay_lease_provider(
        &self,
    ) -> Arc<dyn RemoteDesktopRelayLeaseProvider> {
        Arc::clone(&self.relay_lease_provider)
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn target_monitor_desired_sessions_for_test(
        &self,
    ) -> Vec<String> {
        self.target_monitor.desired_sessions_for_test()
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn track_target_for_test(
        plugin: &Arc<Self>,
        session_id: impl Into<String>,
    ) -> anyhow::Result<()> {
        plugin.target_monitor.track(plugin, session_id.into())
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn crash_target_monitor_generation_for_test(
        &self,
    ) -> anyhow::Result<()> {
        self.target_monitor.crash_generation_for_test()
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn target_monitor_generation_for_test(
        &self,
    ) -> u64 {
        self.target_monitor.generation_for_test()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn persist_recovery_snapshot(
        &self,
        snapshot: &RemoteDesktopRecoverySnapshot,
    ) -> anyhow::Result<std::path::PathBuf> {
        self.recovery.save(snapshot)
    }

    fn rehydrate_recovery_snapshots(plugin: &Arc<Self>) -> anyhow::Result<()> {
        let max_snapshot_rows =
            max_session_rows_for_active_limit(plugin.config().max_sessions())
                .ok_or_else(|| anyhow::anyhow!("RemoteApp recovery row bound overflow"))?;
        let report = plugin.recovery.load_all(max_snapshot_rows)?;
        for rejected in report.rejected() {
            eprintln!(
                "[remote-desktop] ignored recovery snapshot {}: {}",
                rejected.path().display(),
                rejected.reason()
            );
        }
        let snapshots = report.into_snapshots();
        if snapshots.is_empty() {
            return Ok(());
        }
        let mut restored = Vec::new();
        let recovery_now_ms = now_ms();
        for snapshot in snapshots {
            plugin
                .transport_manager()
                .observe_prior_epoch(snapshot.transport_epoch_high_watermark());
            let mut session = match RemoteDesktopSession::rehydrate(&snapshot) {
                Ok(session) => session,
                Err(error) => {
                    eprintln!(
                        "[remote-desktop] ignored recovery snapshot for session {}: {error}",
                        snapshot.session_id()
                    );
                    continue;
                }
            };
            let session_id = session.session_id().to_string();
            let lease_expires_at_ms = session.lease_expires_at_ms();
            let mut relay_refresh_after_ms = None;
            let mut terminal = session.is_terminal();
            let mut terminal_checkpoint_required = false;
            if session.is_terminating() {
                session.finish_recovered_termination(recovery_now_ms);
                terminal = true;
                terminal_checkpoint_required = true;
            } else if !terminal && session.is_expired_at(recovery_now_ms) {
                session.expire(recovery_now_ms);
                terminal = true;
                terminal_checkpoint_required = true;
            }
            if !terminal {
                let relay_lease = match plugin
                    .relay_lease_provider()
                    .acquire(&session_id, session.subject_ura())
                {
                    Ok(relay_lease) => relay_lease,
                    Err(error) => {
                        eprintln!(
                            "[remote-desktop] ignored recovery snapshot for session {session_id}: failed to reacquire Hub relay lease: {error}"
                        );
                        continue;
                    }
                };
                session.install_relay_lease(relay_lease);
                relay_refresh_after_ms = session
                    .active_relay_lease()
                    .map(|lease| lease.refresh_after_ms());
            }
            let recovery_snapshot = if terminal_checkpoint_required {
                match RemoteDesktopRecoverySnapshot::from_session(&session) {
                    Ok(snapshot) => Some(snapshot),
                    Err(error) => {
                        eprintln!(
                            "[remote-desktop] ignored recovery snapshot for session {session_id}: failed to derive recovered terminal checkpoint: {error}"
                        );
                        continue;
                    }
                }
            } else {
                None
            };
            if let Some(recovery_snapshot) = recovery_snapshot {
                if let Err(error) = plugin.persist_recovery_snapshot(&recovery_snapshot) {
                    eprintln!(
                        "[remote-desktop] ignored recovery snapshot for session {session_id}: failed to persist recovered terminal checkpoint: {error}"
                    );
                    continue;
                }
            }
            plugin.session_store().with_sessions(|sessions| {
                sessions.insert(session_id.clone(), session);
            });
            if !terminal {
                if let Err(error) =
                    Self::schedule_session_lease(plugin, session_id.clone(), lease_expires_at_ms)
                {
                    Self::remove_rehydrated_session(plugin, &session_id);
                    eprintln!(
                        "[remote-desktop] ignored recovery snapshot for session {session_id}: failed to schedule recovered lease: {error}"
                    );
                    continue;
                }
                if let Some(refresh_after_ms) = relay_refresh_after_ms {
                    if let Err(error) = Self::schedule_relay_lease_refresh(
                        plugin,
                        session_id.clone(),
                        refresh_after_ms,
                    ) {
                        plugin.cancel_session_lease(&session_id);
                        Self::remove_rehydrated_session(plugin, &session_id);
                        eprintln!(
                            "[remote-desktop] ignored recovery snapshot for session {session_id}: failed to schedule Hub relay refresh: {error}"
                        );
                        continue;
                    }
                }
                if let Err(error) = Self::track_session_target(plugin, session_id.clone()) {
                    plugin.cancel_session_lease(&session_id);
                    plugin.cancel_session_target_tracking(&session_id);
                    Self::remove_rehydrated_session(plugin, &session_id);
                    eprintln!(
                        "[remote-desktop] ignored recovery snapshot for session {session_id}: failed to track recovered target: {error}"
                    );
                    continue;
                }
            }
            restored.push(session_id);
        }
        if !restored.is_empty() {
            eprintln!(
                "[remote-desktop] rehydrated {} recovery snapshot(s): {}",
                restored.len(),
                restored.join(",")
            );
        }
        Ok(())
    }

    fn remove_rehydrated_session(plugin: &RemoteDesktopPlugin, session_id: &str) {
        let relay_lease = plugin.session_store().with_sessions(|sessions| {
            sessions
                .remove(session_id)
                .and_then(|session| session.active_relay_lease().cloned())
        });
        if let Some(relay_lease) = relay_lease {
            if let Err(error) = plugin.relay_lease_provider().release(&relay_lease) {
                eprintln!(
                    "[remote-desktop] failed to release relay lease {} after recovery rollback: {error}",
                    relay_lease.lease_id()
                );
            }
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn screen_backend(
        &self,
    ) -> Arc<dyn ScreenSnapshotBackend> {
        Arc::clone(&self.screen_backend)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_binding_verifier(
        &self,
    ) -> Arc<dyn RemoteAppTargetBindingVerifier> {
        Arc::clone(&self.target_binding_verifier)
    }
}

fn retry_relay_refresh_deadline(
    plugin: &RemoteDesktopPlugin,
    session_id: &str,
    current_lease_id: &str,
    expires_at_ms: u64,
    reason: &str,
) -> Option<u64> {
    let now = now_ms();
    if expires_at_ms <= now {
        let expired = plugin.session_store().with_sessions(|sessions| {
            sessions.get_mut(session_id).and_then(|session| {
                session.retire_relay_lease_if_current(current_lease_id, "hub_relay_refresh_expired")
            })
        });
        if let Some(expired) = expired {
            release_relay_lease(
                plugin.relay_lease_provider().as_ref(),
                &expired,
                "after refresh authorization expired",
            );
        }
        eprintln!(
            "[remote-desktop] Hub relay lease expired for {session_id} after refresh failure: {reason}"
        );
        return None;
    }
    eprintln!("[remote-desktop] Hub relay lease refresh pending for {session_id}: {reason}");
    Some(now.saturating_add(10_000).min(expires_at_ms))
}

fn commit_relay_lease_refresh(
    plugin: &RemoteDesktopPlugin,
    session_id: &str,
    expected_lease_id: &str,
    refreshed: crate::daemon::plugins::remote_desktop::relay_lease::RemoteDesktopRelayLease,
) -> Option<u64> {
    let next_refresh = refreshed.refresh_after_ms();
    let rotation =
        plugin
            .session_store()
            .with_sessions(|sessions| match sessions.get_mut(session_id) {
                Some(session) => {
                    session.rotate_relay_lease_if_current(expected_lease_id, refreshed)
                }
                None => RemoteDesktopRelayLeaseRotation::Unowned(refreshed),
            });
    match rotation {
        RemoteDesktopRelayLeaseRotation::Installed => Some(next_refresh),
        RemoteDesktopRelayLeaseRotation::AlreadyOwned => None,
        RemoteDesktopRelayLeaseRotation::Unowned(unattached) => {
            release_relay_lease(
                plugin.relay_lease_provider().as_ref(),
                &unattached,
                "after refresh lost its session owner",
            );
            None
        }
    }
}

fn release_relay_lease(
    provider: &dyn RemoteDesktopRelayLeaseProvider,
    lease: &crate::daemon::plugins::remote_desktop::relay_lease::RemoteDesktopRelayLease,
    context: &str,
) {
    if let Err(error) = provider.release(lease) {
        eprintln!(
            "[remote-desktop] Hub relay lease {} release failed {context}: {error}",
            lease.lease_id()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use serde_json::json;

    use crate::daemon::ability::builtins::resources::media::screen_snapshot::SyntheticScreenBackend;
    use crate::daemon::ability::dispatch::StreamSource;
    use crate::daemon::persistence::resources::{self, ResourcesFile};
    use crate::daemon::plugins::remote_desktop::constants::REASON_SESSION_EXPIRED;
    use crate::daemon::plugins::remote_desktop::handlers::{
        create_session, end_session, show_session, watch_events,
    };
    use crate::daemon::plugins::remote_desktop::relay_lease::{
        RemoteDesktopRelayLease, RemoteDesktopRelayLeaseInit, EASYNET_RELAY_PROVIDER,
    };
    use crate::daemon::plugins::remote_desktop::session_recovery::RemoteDesktopRecoverySnapshot;
    use crate::daemon::plugins::remote_desktop::test_support::{
        create_test_session, env_for, reset_store, seed_display, test_lock, test_runtime_limits,
        test_session_init, with_input_control_consent_ticket, TestRemoteAppTargetBindingVerifier,
    };

    #[derive(Default)]
    struct ReleaseRecordingRelayProvider {
        released_lease_ids: Mutex<Vec<String>>,
    }

    impl RemoteDesktopRelayLeaseProvider for ReleaseRecordingRelayProvider {
        fn acquire(
            &self,
            _session_id: &str,
            _resource_ura: &str,
        ) -> anyhow::Result<RemoteDesktopRelayLeaseAvailability> {
            Ok(RemoteDesktopRelayLeaseAvailability::unavailable(
                "test_acquire_not_used",
            ))
        }

        fn release(&self, lease: &RemoteDesktopRelayLease) -> anyhow::Result<()> {
            self.released_lease_ids
                .lock()
                .expect("released lease ids lock")
                .push(lease.lease_id().to_string());
            Ok(())
        }
    }

    fn refresh_test_lease(
        lease_id: &str,
        session_id: &str,
        resource_ura: &str,
    ) -> RemoteDesktopRelayLease {
        RemoteDesktopRelayLease::from_init(
            session_id,
            resource_ura,
            RemoteDesktopRelayLeaseInit {
                provider: EASYNET_RELAY_PROVIDER.to_string(),
                lease_id: lease_id.to_string(),
                session_id: session_id.to_string(),
                device_ura: "easynet:///r/acme/device/01DEV".to_string(),
                resource_ura: resource_ura.to_string(),
                urls: vec!["turn:relay.example.test:3478?transport=udp".to_string()],
                username: "refresh-user".to_string(),
                credential: "refresh-secret".to_string(),
                issued_at_ms: 1,
                refresh_after_ms: 2,
                expires_at_ms: 3,
            },
        )
        .expect("test refresh lease")
    }

    #[test]
    fn refresh_result_without_a_session_owner_releases_current_hub_lease() {
        let provider = Arc::new(ReleaseRecordingRelayProvider::default());
        let plugin = RemoteDesktopPlugin::with_relay_lease_provider_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
            provider.clone(),
        );
        let session_id = "rd-refresh-owner-race";
        let resource_ura = "easynet:///r/acme/resource/device.01DEV/streams/window.42";
        let refreshed = refresh_test_lease("lease-refresh-owner-race", session_id, resource_ura);

        assert_eq!(
            commit_relay_lease_refresh(&plugin, session_id, "lease-prior", refreshed),
            None
        );
        assert_eq!(
            provider
                .released_lease_ids
                .lock()
                .expect("released lease ids lock")
                .as_slice(),
            ["lease-refresh-owner-race"]
        );
    }

    #[test]
    fn idempotent_duplicate_refresh_does_not_release_session_owned_hub_lease() {
        let provider = Arc::new(ReleaseRecordingRelayProvider::default());
        let plugin = RemoteDesktopPlugin::with_relay_lease_provider_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
            provider.clone(),
        );
        let session_id = "rd-refresh-duplicate";
        let resource_ura = "easynet:///r/acme/resource/display.relay-refresh";
        let mut session = RemoteDesktopSession::new(test_session_init(
            session_id,
            resource_ura,
            vec!["webrtc".into()],
        ));
        session.install_relay_lease(RemoteDesktopRelayLeaseAvailability::Active(
            refresh_test_lease("lease-refresh-current", session_id, resource_ura),
        ));
        plugin.session_store().with_sessions(|sessions| {
            sessions.insert(session_id.to_string(), session);
        });

        assert_eq!(
            commit_relay_lease_refresh(
                &plugin,
                session_id,
                "lease-refresh-prior",
                refresh_test_lease("lease-refresh-current", session_id, resource_ura),
            ),
            None
        );
        assert!(
            provider
                .released_lease_ids
                .lock()
                .expect("released lease ids lock")
                .is_empty(),
            "an idempotent duplicate refresh is still owned by the session"
        );
        assert_eq!(
            plugin.session_store().with_sessions(|sessions| {
                sessions
                    .get(session_id)
                    .and_then(RemoteDesktopSession::active_relay_lease)
                    .map(RemoteDesktopRelayLease::lease_id)
                    .map(str::to_string)
            }),
            Some("lease-refresh-current".to_string())
        );
    }

    #[test]
    fn plugin_startup_rehydrates_recovery_snapshot_for_public_show_session() {
        let _lock = test_lock();
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("temp recovery dir");
        let recovery = Arc::new(RemoteDesktopRecoveryStore::new(temp.path().to_path_buf()));
        let source = RemoteDesktopPlugin::with_target_binding_verifier(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
        );
        reset_store(&source);
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-startup-rehydrate-display");
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let created = create_session::handle(
            Arc::clone(&source),
            env.clone(),
            with_input_control_consent_ticket(
                &source,
                &env,
                json!({
                    "session_id": "rd-startup-rehydrate",
                    "mode": "interactive",
                    "input_policy": {
                        "keyboard_enabled": true,
                        "pointer_enabled": true
                    }
                }),
            ),
        )
        .expect("source session creates");
        let token = created["session_token"]
            .as_str()
            .expect("create_session returns token")
            .to_string();
        let snapshot = source
            .session_store()
            .with_sessions(|sessions| {
                RemoteDesktopRecoverySnapshot::from_session(
                    sessions
                        .get("rd-startup-rehydrate")
                        .expect("source session exists"),
                )
            })
            .expect("snapshot derives from source session");
        let snapshot = RemoteDesktopRecoverySnapshot::new(
            snapshot.session_id().to_string(),
            snapshot.session_token().to_string(),
            snapshot.creator_caller_ura().to_string(),
            snapshot.selected_resource_ura().to_string(),
            snapshot.subject_display_name().to_string(),
            snapshot.target_binding().clone(),
            snapshot.consent().clone(),
            snapshot.mode().to_string(),
            snapshot.transport_preferences().to_vec(),
            snapshot.video().clone(),
            snapshot.input_policy().clone(),
            snapshot.created_at_ms(),
            snapshot.updated_at_ms(),
            snapshot.lease_expires_at_ms(),
            snapshot.lifecycle_state().to_string(),
            snapshot.termination_reason().map(ToString::to_string),
            Some("accessibility_permission_denied".to_string()),
            snapshot.terminal_receipt(),
            snapshot.events(),
        )
        .expect("snapshot with runtime input block reason stays valid");
        recovery.save(&snapshot).expect("snapshot saves");
        let mut malformed = serde_json::to_value(&snapshot).expect("snapshot serializes");
        malformed["session_id"] = json!("aa-malformed-target-tracking");
        malformed["target_tracking"] = json!({});
        std::fs::write(
            temp.path().join("aa-malformed-target-tracking.json"),
            serde_json::to_vec_pretty(&malformed).expect("malformed fixture serializes"),
        )
        .expect("write semantically malformed target tracking fixture");
        std::fs::write(temp.path().join("rd-corrupt.json"), b"{not json")
            .expect("write corrupt snapshot fixture");

        let recovered = RemoteDesktopPlugin::with_recovery_store_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
            Arc::clone(&recovery),
        );

        let shown = show_session::handle(
            Arc::clone(&recovered),
            env.clone(),
            json!({
                "session_id": "rd-startup-rehydrate",
                "session_token": token,
            }),
        )
        .expect("show_session must use the rehydrated session row");
        assert_eq!(shown["session_id"], json!("rd-startup-rehydrate"));
        assert_eq!(shown["state"], json!("degraded"));
        assert_eq!(shown["media_transport_ready"], json!(false));
        assert_eq!(
            shown["input_readiness"]["blocked_reason"],
            json!("accessibility_permission_denied")
        );
        assert_eq!(shown["input_readiness"], shown["input_plane"]["readiness"]);
        assert_eq!(
            recovered.target_monitor_desired_sessions_for_test(),
            vec!["rd-startup-rehydrate".to_string()],
            "rehydrated non-terminal sessions must re-enter target monitoring"
        );
        let shown_rehydrated_event = shown["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["event_type"] == json!("SESSION_REHYDRATED"))
            .expect("show_session projects the rehydrated event");
        assert_eq!(
            shown_rehydrated_event["recoverability"],
            json!("retry_session")
        );
        assert_eq!(shown_rehydrated_event["subject_ura"], json!(ura));
        assert!(
            shown_rehydrated_event["target_identity_epoch"]
                .as_u64()
                .unwrap_or(0)
                > 0
        );
        assert!(shown_rehydrated_event["payload"]["target_binding"]["subject_ura"] == json!(ura));

        let events = watch_events::handle(
            Arc::clone(&recovered),
            env.clone(),
            json!({
                "session_id": "rd-startup-rehydrate",
                "session_token": token,
                "from_sequence": 0,
            }),
        )
        .expect("watch_events must replay the rehydrated session row");
        let replayed = match events {
            StreamSource::SnapshotThenLive(events, _) => events,
            _ => panic!("rehydrated non-terminal session must replay then remain live"),
        };
        let replayed_rehydrated_event = replayed
            .iter()
            .find(|event| event["event_type"] == json!("SESSION_REHYDRATED"))
            .expect("watch_events replays the rehydrated event");
        assert_eq!(replayed_rehydrated_event["subject_ura"], json!(ura));
        assert!(
            replayed_rehydrated_event["target_geometry_revision"]
                .as_u64()
                .unwrap_or(0)
                > 0
        );

        let ended = end_session::handle(
            Arc::clone(&recovered),
            env,
            json!({
                "session_id": "rd-startup-rehydrate",
                "session_token": created["session_token"],
                "reason": "rehydrate_test_cleanup",
            }),
        )
        .expect("end_session must close the rehydrated session row");
        assert_eq!(ended["state"], json!("closed"));
        assert_eq!(ended["end_reason"], json!("rehydrate_test_cleanup"));
    }

    #[test]
    fn plugin_startup_completes_durable_closing_intent_without_rescheduling_it() {
        let _lock = test_lock();
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("temp recovery dir");
        let recovery = Arc::new(RemoteDesktopRecoveryStore::new(temp.path().to_path_buf()));
        let source = RemoteDesktopPlugin::with_target_binding_verifier(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
        );
        reset_store(&source);
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-startup-closing-display");
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let created = create_test_session(
            Arc::clone(&source),
            env.clone(),
            json!({"session_id": "rd-startup-closing", "mode": "view_only"}),
        )
        .expect("source session creates");
        let token = created["session_token"]
            .as_str()
            .expect("create_session returns token")
            .to_string();
        let snapshot = source
            .session_store()
            .with_sessions(|sessions| {
                let session = sessions
                    .get_mut("rd-startup-closing")
                    .expect("source session exists");
                assert!(session.begin_close("caller_ended"));
                RemoteDesktopRecoverySnapshot::from_session(session)
            })
            .expect("Closing snapshot derives");
        recovery.save(&snapshot).expect("Closing snapshot saves");
        drop(source);

        let recovered = RemoteDesktopPlugin::with_recovery_store_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
            Arc::clone(&recovery),
        );
        let shown = show_session::handle(
            Arc::clone(&recovered),
            env,
            json!({
                "session_id": "rd-startup-closing",
                "session_token": token,
            }),
        )
        .expect("recovered Closing session remains inspectable");

        assert_eq!(shown["state"], json!("closed"));
        assert_eq!(shown["end_reason"], json!("caller_ended"));
        assert_eq!(
            shown["terminal_receipt"]["reason_code"],
            json!("caller_ended")
        );
        assert!(recovered
            .target_monitor_desired_sessions_for_test()
            .is_empty());
        let persisted = recovery
            .load("rd-startup-closing")
            .expect("terminal snapshot loads")
            .expect("terminal snapshot persists");
        assert_eq!(persisted.lifecycle_state(), "closed");
        assert!(persisted.terminal_receipt().is_some());
    }

    #[test]
    fn plugin_startup_expires_recovery_snapshot_that_lapsed_while_daemon_was_down() {
        let _lock = test_lock();
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("temp recovery dir");
        let recovery = Arc::new(RemoteDesktopRecoveryStore::new(temp.path().to_path_buf()));
        let source = RemoteDesktopPlugin::with_target_binding_verifier(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
        );
        reset_store(&source);
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-startup-expired-display");
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let created = create_test_session(
            Arc::clone(&source),
            env.clone(),
            json!({"session_id": "rd-startup-expired", "mode": "view_only"}),
        )
        .expect("source session creates");
        let token = created["session_token"]
            .as_str()
            .expect("create_session returns token")
            .to_string();
        let snapshot = source
            .session_store()
            .with_sessions(|sessions| {
                RemoteDesktopRecoverySnapshot::from_session(
                    sessions
                        .get("rd-startup-expired")
                        .expect("source session exists"),
                )
            })
            .expect("snapshot derives from source session");
        let expired_snapshot = RemoteDesktopRecoverySnapshot::new(
            snapshot.session_id().to_string(),
            snapshot.session_token().to_string(),
            snapshot.creator_caller_ura().to_string(),
            snapshot.selected_resource_ura().to_string(),
            snapshot.subject_display_name().to_string(),
            snapshot.target_binding().clone(),
            snapshot.consent().clone(),
            snapshot.mode().to_string(),
            snapshot.transport_preferences().to_vec(),
            snapshot.video().clone(),
            snapshot.input_policy().clone(),
            1,
            1,
            1,
            snapshot.lifecycle_state().to_string(),
            snapshot.termination_reason().map(ToString::to_string),
            snapshot
                .input_runtime_block_reason()
                .map(ToString::to_string),
            snapshot.terminal_receipt(),
            snapshot.events(),
        )
        .expect("expired snapshot remains schema-valid");
        recovery
            .save(&expired_snapshot)
            .expect("expired snapshot saves");

        let recovered = RemoteDesktopPlugin::with_recovery_store_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
            Arc::clone(&recovery),
        );

        let shown = show_session::handle(
            Arc::clone(&recovered),
            env,
            json!({
                "session_id": "rd-startup-expired",
                "session_token": token,
            }),
        )
        .expect("expired recovery row remains inspectable");
        assert_eq!(shown["session_id"], json!("rd-startup-expired"));
        assert_eq!(shown["state"], json!("closed"));
        assert_eq!(shown["end_reason"], json!(REASON_SESSION_EXPIRED));
        assert_eq!(
            shown["terminal_receipt"]["reason_code"],
            json!(REASON_SESSION_EXPIRED)
        );
        assert!(
            recovered
                .target_monitor_desired_sessions_for_test()
                .is_empty(),
            "expired recovery rows must not re-enter target monitoring"
        );
        let persisted = recovery
            .load("rd-startup-expired")
            .expect("load persisted terminal snapshot")
            .expect("terminal snapshot exists");
        assert!(
            persisted.terminal_receipt().is_some(),
            "startup expiry must be durable"
        );
    }
}
