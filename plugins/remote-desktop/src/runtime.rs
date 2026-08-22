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
use crate::daemon::plugins::remote_desktop::session::{now_ms, RemoteDesktopSession};
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
        Self::with_target_binding_verifier_inner(
            screen_backend,
            Arc::new(PlatformRemoteAppTargetBindingVerifier),
            config,
            Arc::new(RemoteDesktopRecoveryStore::daemon_default()),
            true,
        )
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn with_target_binding_verifier(
        screen_backend: Arc<dyn ScreenSnapshotBackend>,
        target_binding_verifier: Arc<dyn RemoteAppTargetBindingVerifier>,
        config: RemoteDesktopRuntimeConfig,
    ) -> Arc<Self> {
        Self::with_target_binding_verifier_inner(
            screen_backend,
            target_binding_verifier,
            config,
            Arc::new(RemoteDesktopRecoveryStore::daemon_default()),
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
        Self::with_target_binding_verifier_inner(
            screen_backend,
            target_binding_verifier,
            config,
            recovery,
            true,
        )
    }

    fn with_target_binding_verifier_inner(
        screen_backend: Arc<dyn ScreenSnapshotBackend>,
        target_binding_verifier: Arc<dyn RemoteAppTargetBindingVerifier>,
        config: RemoteDesktopRuntimeConfig,
        recovery: Arc<RemoteDesktopRecoveryStore>,
        rehydrate: bool,
    ) -> Arc<Self> {
        let plugin = Arc::new(Self {
            sessions: Arc::new(RemoteDesktopSessionStore::new()),
            consent: Arc::new(RemoteDesktopConsentRegistry::new(
                config.max_sessions().saturating_mul(4),
            )),
            lease_monitor: Arc::new(RemoteDesktopLeaseMonitor::new()),
            target_monitor: Arc::new(RemoteDesktopTargetMonitor::new()),
            transports: Arc::new(RemoteDesktopTransportManager::new()),
            recovery,
            screen_backend,
            target_binding_verifier,
            config,
        });
        if rehydrate {
            if let Err(err) = Self::rehydrate_recovery_snapshots(&plugin) {
                eprintln!("[remote-desktop] recovery snapshot rehydration failed: {err}");
            }
        }
        plugin
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

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn target_monitor_desired_sessions_for_test(
        &self,
    ) -> Vec<String> {
        self.target_monitor.desired_sessions_for_test()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn persist_recovery_snapshot(
        &self,
        snapshot: &RemoteDesktopRecoverySnapshot,
    ) -> anyhow::Result<std::path::PathBuf> {
        self.recovery.save(snapshot)
    }

    fn rehydrate_recovery_snapshots(plugin: &Arc<Self>) -> anyhow::Result<()> {
        let report = plugin.recovery.load_all()?;
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
            let mut session = RemoteDesktopSession::rehydrate(&snapshot)?;
            let session_id = session.session_id().to_string();
            let lease_expires_at_ms = session.lease_expires_at_ms();
            let mut terminal = session.is_terminal();
            if !terminal && session.is_expired_at(recovery_now_ms) {
                session.expire(recovery_now_ms);
                terminal = true;
            }
            let recovery_snapshot = if terminal {
                Some(RemoteDesktopRecoverySnapshot::from_session(&session)?)
            } else {
                None
            };
            plugin.session_store().with_sessions(|sessions| {
                sessions.insert(session_id.clone(), session);
            });
            if let Some(recovery_snapshot) = recovery_snapshot {
                plugin.persist_recovery_snapshot(&recovery_snapshot)?;
            }
            if !terminal {
                Self::schedule_session_lease(plugin, session_id.clone(), lease_expires_at_ms)?;
                Self::track_session_target(plugin, session_id.clone())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::daemon::ability::builtins::resources::media::screen_snapshot::SyntheticScreenBackend;
    use crate::daemon::ability::dispatch::StreamSource;
    use crate::daemon::persistence::resources::{self, ResourcesFile};
    use crate::daemon::plugins::remote_desktop::constants::REASON_SESSION_EXPIRED;
    use crate::daemon::plugins::remote_desktop::handlers::{
        end_session, show_session, watch_events,
    };
    use crate::daemon::plugins::remote_desktop::session_recovery::RemoteDesktopRecoverySnapshot;
    use crate::daemon::plugins::remote_desktop::test_support::{
        create_test_session, env_for, reset_store, seed_display, test_lock, test_runtime_limits,
        TestRemoteAppTargetBindingVerifier,
    };

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
        let created = create_test_session(
            Arc::clone(&source),
            env.clone(),
            json!({"session_id": "rd-startup-rehydrate", "mode": "view_only"}),
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
        recovery.save(&snapshot).expect("snapshot saves");
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
            recovered.target_monitor_desired_sessions_for_test(),
            vec!["rd-startup-rehydrate".to_string()],
            "rehydrated non-terminal sessions must re-enter target monitoring"
        );
        assert!(shown["events"].as_array().unwrap().iter().any(|event| {
            event["event_type"] == json!("SESSION_REHYDRATED")
                && event["recoverability"] == json!("retry_session")
        }));

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
        assert!(replayed
            .iter()
            .any(|event| event["event_type"] == json!("SESSION_REHYDRATED")));

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
