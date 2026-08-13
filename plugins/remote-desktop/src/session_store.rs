// EasyNet CLI — remote desktop session store
// ===========================================
//
// File: plugins/remote-desktop/src/session_store.rs
// Description: Synchronized session map and store-level transport projections.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use serde_json::Value;

use crate::daemon::plugins::remote_desktop::constants::DIRECT_WEBRTC_ENDPOINT_PREFIX;
use crate::daemon::plugins::remote_desktop::sdp::ice_candidate_text;
use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::target::RemoteAppTargetBinding;
use crate::daemon::plugins::remote_desktop::target_tracking::{
    TargetObservation, TargetTrackerSnapshot,
};

pub(in crate::daemon::plugins::remote_desktop) struct TargetObservationInputs {
    pub(in crate::daemon::plugins::remote_desktop) binding: RemoteAppTargetBinding,
    pub(in crate::daemon::plugins::remote_desktop) snapshot: TargetTrackerSnapshot,
    pub(in crate::daemon::plugins::remote_desktop) binding_id: String,
    pub(in crate::daemon::plugins::remote_desktop) binding_epoch: u64,
}

/// Runtime-owned synchronized map of remote desktop sessions.
///
/// Invariant 1: callers mutate session rows only while holding the store lock.
/// Invariant 2: transport callbacks enter through store-level projection
/// methods, so transport code never reaches into session internals.
/// Invariant 3: poisoned mutexes are recovered because a daemon-side panic in
/// one handler must not permanently brick unrelated session cleanup.
#[derive(Debug, Default)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopSessionStore {
    inner: Mutex<HashMap<String, RemoteDesktopSession>>,
}

impl RemoteDesktopSessionStore {
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self::default()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn lock(
        &self,
    ) -> MutexGuard<'_, HashMap<String, RemoteDesktopSession>> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Execute one bounded mutation/read section over the session map without
    /// leaking the mutex guard through the plugin facade.
    pub(in crate::daemon::plugins::remote_desktop) fn with_sessions<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, RemoteDesktopSession>) -> R,
    ) -> R {
        let mut sessions = self.lock();
        f(&mut sessions)
    }

    /// Mark a direct WebRTC media plane ready for one non-terminal session.
    ///
    /// This is a store-level boundary helper: transport code supplies the
    /// session id, while the session model owns the terminal and duplicate
    /// media-ready checks.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_direct_webrtc_media_ready(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
    ) {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        session.mark_webrtc_media_sending(
            epoch,
            format!("{DIRECT_WEBRTC_ENDPOINT_PREFIX}{session_id}"),
        );
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_observation_inputs_for_session(
        &self,
        session_id: &str,
    ) -> Option<TargetObservationInputs> {
        let sessions = self.lock();
        let session = sessions.get(session_id)?;
        if session.is_terminal() {
            return None;
        }
        let binding = session.target_binding().clone();
        let binding_id = binding.binding_id().to_string();
        let binding_epoch = binding.binding_epoch();
        Some(TargetObservationInputs {
            binding,
            snapshot: session.target_snapshot().clone(),
            binding_id,
            binding_epoch,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn record_target_observation_for_session(
        &self,
        session_id: &str,
        binding_id: &str,
        binding_epoch: u64,
        observation: TargetObservation,
    ) {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        let binding = session.target_binding();
        if session.is_terminal()
            || binding.binding_id() != binding_id
            || binding.binding_epoch() != binding_epoch
        {
            return;
        }
        session.record_target_observation(observation);
    }

    /// Mark a direct WebRTC endpoint failed for one non-terminal session.
    ///
    /// This helper intentionally accepts domain strings rather than transport
    /// error types so the session store stays independent of WebRTC internals.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_direct_webrtc_failed(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
        reason: &str,
        message: String,
    ) {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        if let Some(preview_stop) = session.mark_webrtc_failed(epoch, reason, message) {
            let _ = preview_stop.send(true);
        }
    }

    /// Append a local ICE candidate projected from the transport layer.
    ///
    /// Empty candidates are ignored before mutating state because they
    /// represent end-of-candidates markers, not a device candidate to publish.
    pub(in crate::daemon::plugins::remote_desktop) fn record_local_webrtc_candidate(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
        candidate: Value,
    ) -> anyhow::Result<()> {
        if ice_candidate_text(&candidate)?.trim().is_empty() {
            return Ok(());
        }
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return Ok(());
        };
        if session.transport_epoch() != Some(epoch.value()) {
            return Ok(());
        }
        session.record_local_ice_candidate(candidate);
        Ok(())
    }

    /// Record a WebRTC diagnostic event projected into session-state terms.
    pub(in crate::daemon::plugins::remote_desktop) fn record_webrtc_diagnostic(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
        event_type: &str,
        error: Option<String>,
        payload: Value,
    ) {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        if session.transport_epoch() != Some(epoch.value()) {
            return;
        }
        session.record_webrtc_diagnostic(event_type, error, payload);
    }

    /// Store latest media stats for one non-terminal session.
    #[cfg(target_os = "macos")]
    pub(in crate::daemon::plugins::remote_desktop) fn record_media_pipeline_stats(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
        stats: Value,
    ) {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        session.record_media_stats(epoch, stats);
    }

    /// Detach the diagnostic InvokeBidi preview transport after a worker
    /// reaches a normal terminal path such as client close or stream end.
    pub(in crate::daemon::plugins::remote_desktop) fn detach_preview_transport_from_worker(
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
    pub(in crate::daemon::plugins::remote_desktop) fn mark_preview_transport_failed(
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::daemon::plugins::remote_desktop::constants::TRANSPORT_WEBRTC;
    use crate::daemon::plugins::remote_desktop::test_support::test_session_init;

    fn insert_test_session(store: &RemoteDesktopSessionStore, session_id: &str) {
        store.with_sessions(|sessions| {
            let mut session = RemoteDesktopSession::new(test_session_init(
                session_id,
                "easynet:///r/acme/resource/display.01",
                vec![TRANSPORT_WEBRTC.to_string()],
            ));
            session.begin_webrtc_negotiation(TransportEpoch::new(1));
            sessions.insert(session_id.to_string(), session);
        });
    }

    #[test]
    fn local_webrtc_candidate_rejects_schema_incomplete_rows() {
        let store = RemoteDesktopSessionStore::new();
        insert_test_session(&store, "rd-local-candidate-schema");

        for (candidate, expected) in [
            (json!("candidate:1"), "must be an object or null"),
            (json!({}), "must include string `candidate`"),
            (json!({"candidate": 7}), "must include string `candidate`"),
        ] {
            let err = store
                .record_local_webrtc_candidate(
                    "rd-local-candidate-schema",
                    TransportEpoch::new(1),
                    candidate,
                )
                .expect_err("malformed local ICE candidate must fail closed")
                .to_string();
            assert!(err.contains(expected), "expected {expected:?}; got {err}");
        }

        store.with_sessions(|sessions| {
            let session = sessions.get("rd-local-candidate-schema").unwrap();
            assert!(
                session.local_ice_candidates().is_empty(),
                "malformed local candidates must not enter session signaling"
            );
        });
    }

    #[test]
    fn local_webrtc_candidate_records_only_non_empty_candidates() {
        let store = RemoteDesktopSessionStore::new();
        insert_test_session(&store, "rd-local-candidate-ok");

        store
            .record_local_webrtc_candidate(
                "rd-local-candidate-ok",
                TransportEpoch::new(1),
                json!({"candidate": "", "sdpMid": "0", "sdpMLineIndex": 0}),
            )
            .expect("explicit end marker is accepted");
        store
            .record_local_webrtc_candidate(
                "rd-local-candidate-ok",
                TransportEpoch::new(1),
                json!({
                    "candidate": "candidate:1 1 UDP 2122252543 abc.local 54400 typ host",
                    "sdpMid": "0",
                    "sdpMLineIndex": 0
                }),
            )
            .expect("candidate records");

        store.with_sessions(|sessions| {
            let session = sessions.get("rd-local-candidate-ok").unwrap();
            let candidates = session.local_ice_candidates();
            assert_eq!(candidates.len(), 1);
            assert_eq!(
                candidates[0]["candidate"],
                json!("candidate:1 1 UDP 2122252543 abc.local 54400 typ host")
            );
        });
    }

    #[test]
    fn direct_webrtc_media_ready_is_idempotent() {
        let store = RemoteDesktopSessionStore::new();
        insert_test_session(&store, "rd-media-ready");

        store.mark_direct_webrtc_media_ready("rd-media-ready", TransportEpoch::new(1));
        store.mark_direct_webrtc_media_ready("rd-media-ready", TransportEpoch::new(1));

        store.with_sessions(|sessions| {
            let session = sessions.get("rd-media-ready").unwrap();
            assert!(session.media_transport_ready());
            let connected_events = session
                .events()
                .into_iter()
                .filter(|event| event["event_type"] == json!("MEDIA_SENDER_READY"))
                .count();
            assert_eq!(connected_events, 1);
        });
    }

    #[test]
    fn peer_connection_diagnostic_does_not_mark_media_ready() {
        let store = RemoteDesktopSessionStore::new();
        insert_test_session(&store, "rd-peer-connected-only");

        store.record_webrtc_diagnostic(
            "rd-peer-connected-only",
            TransportEpoch::new(1),
            "PEER_CONNECTION_STATE_CHANGED",
            None,
            json!({ "peer_connection_state": "connected" }),
        );

        store.with_sessions(|sessions| {
            let session = sessions.get("rd-peer-connected-only").unwrap();
            assert!(!session.media_transport_ready());
            assert!(session
                .events()
                .iter()
                .any(|event| event["event_type"] == json!("PEER_CONNECTION_STATE_CHANGED")));
        });
    }
}
