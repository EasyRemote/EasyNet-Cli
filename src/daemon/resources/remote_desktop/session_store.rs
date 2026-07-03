// EasyNet CLI — remote desktop session store
// ===========================================
//
// File: src/daemon/resources/remote_desktop/session_store.rs
// Description: Synchronized session map and store-level transport projections.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use serde_json::Value;

use crate::daemon::resources::remote_desktop::constants::DIRECT_WEBRTC_ENDPOINT_PREFIX;
use crate::daemon::resources::remote_desktop::session::RemoteDesktopSession;

/// Runtime-owned synchronized map of remote desktop sessions.
///
/// Invariant 1: callers mutate session rows only while holding the store lock.
/// Invariant 2: transport callbacks enter through store-level projection
/// methods, so transport code never reaches into session internals.
/// Invariant 3: poisoned mutexes are recovered because a daemon-side panic in
/// one handler must not permanently brick unrelated session cleanup.
#[derive(Debug, Default)]
pub(in crate::daemon::resources::remote_desktop) struct RemoteDesktopSessionStore {
    inner: Mutex<HashMap<String, RemoteDesktopSession>>,
}

impl RemoteDesktopSessionStore {
    pub(in crate::daemon::resources::remote_desktop) fn new() -> Self {
        Self::default()
    }

    pub(in crate::daemon::resources::remote_desktop) fn lock(
        &self,
    ) -> MutexGuard<'_, HashMap<String, RemoteDesktopSession>> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Execute one bounded mutation/read section over the session map without
    /// leaking the mutex guard through the plugin facade.
    pub(in crate::daemon::resources::remote_desktop) fn with_sessions<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, RemoteDesktopSession>) -> R,
    ) -> R {
        let mut sessions = self.lock();
        f(&mut sessions)
    }

    /// Mark a direct WebRTC endpoint connected for one non-terminal session.
    ///
    /// This is a store-level boundary helper: transport code supplies the
    /// session id, while the session model owns the terminal and duplicate
    /// connection checks.
    pub(in crate::daemon::resources::remote_desktop) fn mark_direct_webrtc_connected(
        &self,
        session_id: &str,
    ) {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        session.mark_webrtc_connected(format!("{DIRECT_WEBRTC_ENDPOINT_PREFIX}{session_id}"));
    }

    /// Mark a direct WebRTC endpoint failed for one non-terminal session.
    ///
    /// This helper intentionally accepts domain strings rather than transport
    /// error types so the session store stays independent of WebRTC internals.
    pub(in crate::daemon::resources::remote_desktop) fn mark_direct_webrtc_failed(
        &self,
        session_id: &str,
        reason: &str,
        message: String,
    ) {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        session.mark_webrtc_failed(reason, message);
    }

    /// Append a local ICE candidate projected from the transport layer.
    ///
    /// Empty candidates are ignored before mutating state because they
    /// represent end-of-candidates markers, not a device candidate to publish.
    pub(in crate::daemon::resources::remote_desktop) fn record_local_webrtc_candidate(
        &self,
        session_id: &str,
        candidate: Value,
    ) {
        if candidate
            .get("candidate")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
        {
            return;
        }
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        session.record_local_ice_candidate(candidate);
    }

    /// Record a WebRTC diagnostic event projected into session-state terms.
    pub(in crate::daemon::resources::remote_desktop) fn record_webrtc_diagnostic(
        &self,
        session_id: &str,
        event_type: &str,
        error: Option<String>,
        payload: Value,
    ) {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        session.record_webrtc_diagnostic(event_type, error, payload);
    }

    /// Store latest media stats for one non-terminal session.
    #[cfg(target_os = "macos")]
    pub(in crate::daemon::resources::remote_desktop) fn record_media_pipeline_stats(
        &self,
        session_id: &str,
        stats: Value,
    ) {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        session.record_media_stats(stats);
    }

    /// Detach the diagnostic InvokeBidi preview transport after a worker
    /// reaches a normal terminal path such as client close or stream end.
    pub(in crate::daemon::resources::remote_desktop) fn detach_preview_transport_from_worker(
        &self,
        session_id: &str,
        reason: &str,
    ) {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        let _ = session.detach_preview_transport_from_worker(reason);
    }

    /// Mark the diagnostic InvokeBidi preview transport failed after capture
    /// or encoding terminates before a clean close.
    pub(in crate::daemon::resources::remote_desktop) fn mark_preview_transport_failed(
        &self,
        session_id: &str,
        reason: &str,
        message: String,
    ) {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        let _ = session.mark_preview_transport_failed(reason, message);
    }
}
