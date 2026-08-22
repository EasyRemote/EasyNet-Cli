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
use crate::daemon::plugins::remote_desktop::session_creation::{
    PlatformRemoteAppTargetBindingVerifier, RemoteAppTargetBindingVerifier,
};
use crate::daemon::plugins::remote_desktop::session_recovery::{
    RemoteDesktopRecoverySnapshot, RemoteDesktopRecoveryStore,
};
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::target_monitor::RemoteDesktopTargetMonitor;
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
    recovery: Arc<RemoteDesktopRecoveryStore>,
    screen_backend: Arc<dyn ScreenSnapshotBackend>,
    target_binding_verifier: Arc<dyn RemoteAppTargetBindingVerifier>,
    config: RemoteDesktopRuntimeConfig,
}

impl RemoteDesktopPlugin {
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        screen_backend: Arc<dyn ScreenSnapshotBackend>,
        config: RemoteDesktopRuntimeConfig,
    ) -> Arc<Self> {
        Self::with_target_binding_verifier(
            screen_backend,
            Arc::new(PlatformRemoteAppTargetBindingVerifier),
            config,
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn with_target_binding_verifier(
        screen_backend: Arc<dyn ScreenSnapshotBackend>,
        target_binding_verifier: Arc<dyn RemoteAppTargetBindingVerifier>,
        config: RemoteDesktopRuntimeConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions: Arc::new(RemoteDesktopSessionStore::new()),
            consent: Arc::new(RemoteDesktopConsentRegistry::new(
                config.max_sessions().saturating_mul(4),
            )),
            lease_monitor: Arc::new(RemoteDesktopLeaseMonitor::new()),
            target_monitor: Arc::new(RemoteDesktopTargetMonitor::new()),
            transports: Arc::new(RemoteDesktopTransportManager::new()),
            recovery: Arc::new(RemoteDesktopRecoveryStore::daemon_default()),
            screen_backend,
            target_binding_verifier,
            config,
        })
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

    pub(in crate::daemon::plugins::remote_desktop) fn consent_registry(
        &self,
    ) -> Arc<RemoteDesktopConsentRegistry> {
        Arc::clone(&self.consent)
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

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn recovery_store(
        &self,
    ) -> Arc<RemoteDesktopRecoveryStore> {
        Arc::clone(&self.recovery)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn persist_recovery_snapshot(
        &self,
        snapshot: &RemoteDesktopRecoverySnapshot,
    ) -> anyhow::Result<std::path::PathBuf> {
        self.recovery.save(snapshot)
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
