// EasyNet CLI — remote desktop session store
// ===========================================
//
// File: plugins/remote-desktop/src/session_store.rs
// Description: Synchronized session map and store-level transport projections.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard};

use serde_json::Value;

use crate::daemon::plugins::remote_desktop::constants::direct_webrtc_endpoint_ura;
use crate::daemon::plugins::remote_desktop::sdp::ice_candidate_text;
use crate::daemon::plugins::remote_desktop::session::{
    RemoteDesktopSession, TargetMediaSourceLost, TargetRebindDeadlineExpiration,
};
use crate::daemon::plugins::remote_desktop::session_events::{
    webrtc_transport_failure_context, WebRtcFailureEventKind,
};
use crate::daemon::plugins::remote_desktop::session_transport_state::{
    ClientMediaFeedback, TransportEpoch,
};
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, ResolvedCaptureTargetProof, TargetResolutionError,
};
use crate::daemon::plugins::remote_desktop::target_tracking::{
    TargetObservation, TargetTrackerSnapshot,
};

pub(in crate::daemon::plugins::remote_desktop) const MAX_TERMINAL_ROWS_PER_ACTIVE_SESSION: usize =
    4;

pub(in crate::daemon::plugins::remote_desktop) struct TargetObservationInputs {
    pub(in crate::daemon::plugins::remote_desktop) binding: RemoteAppTargetBinding,
    pub(in crate::daemon::plugins::remote_desktop) snapshot: TargetTrackerSnapshot,
    pub(in crate::daemon::plugins::remote_desktop) binding_id: String,
    pub(in crate::daemon::plugins::remote_desktop) binding_epoch: u64,
}

pub(in crate::daemon::plugins::remote_desktop) struct TargetObservationCommit {
    pub(in crate::daemon::plugins::remote_desktop) state_changed: bool,
    pub(in crate::daemon::plugins::remote_desktop) media_source_lost: Option<TargetMediaSourceLost>,
}

thread_local! {
    static SESSION_STORE_LOCK_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Mutex guard for the remote desktop session map.
///
/// The wrapper keeps a current-thread lock depth so expensive target/media
/// boundaries can assert they are not running while the session aggregate is
/// locked.
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopSessionStoreGuard<'a> {
    guard: MutexGuard<'a, HashMap<String, RemoteDesktopSession>>,
}

impl<'a> RemoteDesktopSessionStoreGuard<'a> {
    fn new(guard: MutexGuard<'a, HashMap<String, RemoteDesktopSession>>) -> Self {
        SESSION_STORE_LOCK_DEPTH.with(|depth| depth.set(depth.get().saturating_add(1)));
        Self { guard }
    }
}

impl Deref for RemoteDesktopSessionStoreGuard<'_> {
    type Target = HashMap<String, RemoteDesktopSession>;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl DerefMut for RemoteDesktopSessionStoreGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

impl Drop for RemoteDesktopSessionStoreGuard<'_> {
    fn drop(&mut self) {
        SESSION_STORE_LOCK_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
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
    ) -> RemoteDesktopSessionStoreGuard<'_> {
        let guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        RemoteDesktopSessionStoreGuard::new(guard)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn current_thread_lock_depth() -> usize {
        SESSION_STORE_LOCK_DEPTH.with(Cell::get)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn assert_current_thread_unlocked(stage: &str) {
        assert_eq!(
            Self::current_thread_lock_depth(),
            0,
            "{stage} must not run while RemoteDesktopSessionStore is locked"
        );
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

    /// Prune terminal/tombstone rows to the SPEC performance bound `T <= 4S`,
    /// where `S` is the current number of non-terminal sessions.
    ///
    /// This is intentionally a store-level policy instead of a handler-local
    /// cleanup: session lifecycle code decides when a maintenance boundary is
    /// reached, while the session aggregate owns the retention math and oldest
    /// terminal-row selection.
    pub(in crate::daemon::plugins::remote_desktop) fn prune_terminal_rows_to_active_bound_locked(
        sessions: &mut HashMap<String, RemoteDesktopSession>,
    ) -> Vec<String> {
        let active_count = sessions
            .values()
            .filter(|session| !session.is_terminal())
            .count();
        let terminal_limit = active_count.saturating_mul(MAX_TERMINAL_ROWS_PER_ACTIVE_SESSION);
        let mut terminal_rows: Vec<(String, u64)> = sessions
            .iter()
            .filter(|(_, session)| session.is_terminal())
            .map(|(session_id, session)| (session_id.clone(), session.updated_at_ms()))
            .collect();
        if terminal_rows.len() <= terminal_limit {
            return Vec::new();
        }

        let excess = terminal_rows.len() - terminal_limit;
        terminal_rows.sort_by(|(left_id, left_updated_at), (right_id, right_updated_at)| {
            left_updated_at
                .cmp(right_updated_at)
                .then_with(|| left_id.cmp(right_id))
        });
        let removed: Vec<String> = terminal_rows
            .into_iter()
            .take(excess)
            .map(|(session_id, _)| session_id)
            .collect();
        for session_id in &removed {
            sessions.remove(session_id);
        }
        removed
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
        session.mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura(session_id));
    }

    /// Mark the input plane active for a direct WebRTC epoch after policy and
    /// platform input gates have already passed.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_input_channel_ready(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
    ) -> bool {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        session.activate_input_for_transport_epoch(epoch)
    }

    /// Confirm a host-applied input frame for the current direct WebRTC epoch.
    /// This lets the session aggregate clear any runtime input-permission
    /// blocker using execution proof instead of frontend inference.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_input_frame_applied(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
    ) -> bool {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        session.mark_input_frame_applied(epoch)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn mark_input_permission_blocked(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
        reason: &str,
    ) -> bool {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        session.block_input_for_runtime_permission(epoch, reason)
    }

    /// Project WebRTC's transient `disconnected` state as degraded health.
    /// The endpoint remains alive because ICE is allowed to recover without a
    /// new PeerConnection; a later `failed`/`closed` callback retires it.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_direct_webrtc_disconnected(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
    ) -> bool {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        session.report_client_media_state(epoch, "stalled", None)
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

    pub(in crate::daemon::plugins::remote_desktop) fn commit_target_observation_for_session(
        &self,
        session_id: &str,
        binding_id: &str,
        binding_epoch: u64,
        observation: TargetObservation,
    ) -> Option<TargetObservationCommit> {
        let mut sessions = self.lock();
        let session = sessions.get_mut(session_id)?;
        let binding = session.target_binding();
        if session.is_terminal()
            || binding.binding_id() != binding_id
            || binding.binding_epoch() != binding_epoch
        {
            return None;
        }
        let previous_target_snapshot = session.target_snapshot().clone();
        let previous_sequence = session.latest_event_sequence();
        let media_source_lost = session.record_target_observation(observation);
        Some(TargetObservationCommit {
            state_changed: session.target_snapshot() != &previous_target_snapshot
                || session.latest_event_sequence() != previous_sequence,
            media_source_lost,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn expire_target_rebind_deadline_for_session(
        &self,
        session_id: &str,
        binding_id: &str,
        binding_epoch: u64,
        observed_at_ms: u64,
    ) -> Option<TargetRebindDeadlineExpiration> {
        let mut sessions = self.lock();
        let session = sessions.get_mut(session_id)?;
        let binding = session.target_binding();
        if session.is_terminal()
            || binding.binding_id() != binding_id
            || binding.binding_epoch() != binding_epoch
        {
            return None;
        }
        session.expire_target_rebind_deadline(observed_at_ms)
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn pending_media_rebind_binding_for_session(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
        active_media_source_epoch: u64,
    ) -> Option<RemoteAppTargetBinding> {
        let sessions = self.lock();
        let session = sessions.get(session_id)?;
        if session.transport_epoch() != Some(epoch.value()) {
            return None;
        }
        let pending = session.pending_media_rebind_binding()?;
        (pending.media_source_epoch() > active_media_source_epoch).then(|| pending.clone())
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn commit_pending_media_rebind_for_session(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
        binding_epoch: u64,
        media_source_epoch: u64,
        capture_proof: ResolvedCaptureTargetProof,
    ) -> bool {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        session.commit_pending_media_rebind(epoch, binding_epoch, media_source_epoch, capture_proof)
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn supersede_pending_media_rebind_for_session(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
        reason: TargetResolutionError,
        detail: String,
    ) -> bool {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        session.supersede_pending_media_rebind(epoch, reason, detail)
    }

    /// Retire one direct WebRTC generation without terminating its product
    /// session. A later authenticated offer must be able to allocate a newer
    /// epoch and resume the same session identity.
    ///
    /// This helper intentionally accepts domain strings rather than transport
    /// error types so the session store stays independent of WebRTC internals.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_direct_webrtc_generation_failed(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
        reason: &str,
        message: String,
    ) -> bool {
        self.mark_direct_webrtc_generation_failed_with_context(
            session_id,
            epoch,
            WebRtcFailureEventKind::TransportFailed,
            reason,
            message,
            webrtc_transport_failure_context(),
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn mark_direct_webrtc_generation_failed_with_context(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
        event_kind: WebRtcFailureEventKind,
        reason: &str,
        message: String,
        context: Value,
    ) -> bool {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        session
            .mark_webrtc_generation_failed_with_context(epoch, event_kind, reason, message, context)
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
        session.record_local_ice_candidate(candidate)?;
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
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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

    /// Read the latest authenticated browser receiver feedback for one media
    /// generation. The typed copy keeps the encoder loop outside the session
    /// mutex and prevents stale transport epochs from influencing adaptation.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn client_media_feedback_for_session(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
    ) -> Option<ClientMediaFeedback> {
        let sessions = self.lock();
        sessions.get(session_id)?.client_media_feedback(epoch)
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

    use crate::daemon::plugins::remote_desktop::constants::{
        MAX_ICE_CANDIDATE_BYTES, MAX_LOCAL_ICE_CANDIDATES, MAX_SIGNALING_DESCRIPTION_BYTES,
        TRANSPORT_WEBRTC,
    };
    use crate::daemon::plugins::remote_desktop::test_support::test_session_init;
    use crate::daemon::plugins::remote_desktop::view::serialize_session;

    #[test]
    fn lock_depth_tracks_session_store_guard_scope_on_current_thread() {
        let store = RemoteDesktopSessionStore::new();
        assert_eq!(RemoteDesktopSessionStore::current_thread_lock_depth(), 0);

        {
            let _guard = store.lock();
            assert_eq!(RemoteDesktopSessionStore::current_thread_lock_depth(), 1);
        }

        assert_eq!(RemoteDesktopSessionStore::current_thread_lock_depth(), 0);
        RemoteDesktopSessionStore::assert_current_thread_unlocked("remote_desktop.test.unlocked");
    }

    #[test]
    #[should_panic(expected = "remote_desktop.test.locked_boundary")]
    fn unlocked_boundary_assertion_fails_while_session_store_guard_is_held() {
        let store = RemoteDesktopSessionStore::new();
        let _guard = store.lock();

        RemoteDesktopSessionStore::assert_current_thread_unlocked(
            "remote_desktop.test.locked_boundary",
        );
    }

    fn insert_test_session(store: &RemoteDesktopSessionStore, session_id: &str) {
        let mut session = RemoteDesktopSession::new(test_session_init(
            session_id,
            "easynet:///r/acme/resource/display.01",
            vec![TRANSPORT_WEBRTC.to_string()],
        ));
        session.begin_webrtc_negotiation(TransportEpoch::new(1));

        store.with_sessions(|sessions| {
            sessions.insert(session_id.to_string(), session);
        });
    }

    fn test_session(session_id: &str) -> RemoteDesktopSession {
        RemoteDesktopSession::new(test_session_init(
            session_id,
            "easynet:///r/acme/resource/display.01",
            vec![TRANSPORT_WEBRTC.to_string()],
        ))
    }

    #[test]
    fn terminal_rows_are_pruned_to_four_times_active_sessions() {
        let store = RemoteDesktopSessionStore::new();
        let mut seeded_sessions = Vec::new();
        for index in 0..2 {
            let session_id = format!("active-{index}");
            seeded_sessions.push((session_id.clone(), test_session(&session_id)));
        }
        for index in 0..10 {
            let session_id = format!("terminal-{index:02}");
            let mut session = test_session(&session_id);
            session.close("test_terminal");
            seeded_sessions.push((session_id, session));
        }
        store.with_sessions(|sessions| {
            for (session_id, session) in seeded_sessions {
                sessions.insert(session_id, session);
            }

            let removed =
                RemoteDesktopSessionStore::prune_terminal_rows_to_active_bound_locked(sessions);
            assert_eq!(removed.len(), 2);

            let active_count = sessions
                .values()
                .filter(|session| !session.is_terminal())
                .count();
            let terminal_count = sessions
                .values()
                .filter(|session| session.is_terminal())
                .count();
            assert_eq!(active_count, 2);
            assert_eq!(
                terminal_count,
                active_count.saturating_mul(MAX_TERMINAL_ROWS_PER_ACTIVE_SESSION)
            );
        });
    }

    #[test]
    fn terminal_rows_are_removed_when_no_active_sessions_remain() {
        let store = RemoteDesktopSessionStore::new();
        let mut seeded_sessions = Vec::new();
        for index in 0..3 {
            let session_id = format!("terminal-only-{index}");
            let mut session = test_session(&session_id);
            session.close("test_terminal");
            seeded_sessions.push((session_id, session));
        }
        store.with_sessions(|sessions| {
            for (session_id, session) in seeded_sessions {
                sessions.insert(session_id, session);
            }

            let removed =
                RemoteDesktopSessionStore::prune_terminal_rows_to_active_bound_locked(sessions);
            assert_eq!(removed.len(), 3);
            assert!(sessions.is_empty());
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
    fn local_webrtc_candidate_rejects_flood_after_bounded_candidate_cap() {
        let store = RemoteDesktopSessionStore::new();
        insert_test_session(&store, "rd-local-candidate-flood");

        for index in 0..MAX_LOCAL_ICE_CANDIDATES {
            store
                .record_local_webrtc_candidate(
                    "rd-local-candidate-flood",
                    TransportEpoch::new(1),
                    json!({
                        "candidate": format!("candidate:{index} 1 UDP 2122252543 127.0.0.1 {} typ host", 41000 + index),
                        "sdpMid": "0",
                        "sdpMLineIndex": 0
                    }),
                )
                .expect("candidate within cap records");
        }

        let err = store
            .record_local_webrtc_candidate(
                "rd-local-candidate-flood",
                TransportEpoch::new(1),
                json!({
                    "candidate": "candidate:overflow 1 UDP 2122252543 127.0.0.1 49999 typ host",
                    "sdpMid": "0",
                    "sdpMLineIndex": 0
                }),
            )
            .expect_err("candidate over cap must fail closed")
            .to_string();
        assert!(
            err.contains("local ICE candidate cap exceeded"),
            "got {err}"
        );

        store.with_sessions(|sessions| {
            let session = sessions.get("rd-local-candidate-flood").unwrap();
            assert_eq!(
                session.local_ice_candidates().len(),
                MAX_LOCAL_ICE_CANDIDATES,
                "serialized session view must remain bounded at the local candidate cap"
            );
        });
    }

    #[test]
    fn serialized_session_view_remains_bounded_at_signaling_limits() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-serialized-bound",
            "easynet:///r/acme/resource/display.serialized-bound",
            vec![TRANSPORT_WEBRTC.to_string()],
        ));
        assert!(session.begin_webrtc_negotiation(TransportEpoch::new(1)));
        let sdp = format!(
            "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\n{}",
            "a=x\r\n".repeat((MAX_SIGNALING_DESCRIPTION_BYTES / 8).saturating_sub(4096))
        );
        let description = json!({ "type": "answer", "sdp": sdp });
        assert!(
            serde_json::to_vec(&description).unwrap().len() <= MAX_SIGNALING_DESCRIPTION_BYTES,
            "fixture must stay within the accepted SDP description cap"
        );
        session
            .set_description("local", description)
            .expect("max accepted local description records");

        let candidate_pad = "x".repeat(MAX_ICE_CANDIDATE_BYTES.saturating_sub(256));
        for index in 0..MAX_LOCAL_ICE_CANDIDATES {
            let candidate = json!({
                "candidate": format!(
                    "candidate:{index} 1 UDP 2122252543 127.0.0.1 {} typ host {candidate_pad}",
                    41000 + index
                ),
                "sdpMid": "0",
                "sdpMLineIndex": 0
            });
            assert!(
                serde_json::to_vec(&candidate).unwrap().len() <= MAX_ICE_CANDIDATE_BYTES,
                "fixture must stay within the accepted candidate cap"
            );
            session
                .record_local_ice_candidate(candidate)
                .expect("candidate within cap records");
        }

        let view = serialize_session(&session);
        assert_eq!(
            view["signaling"]["local_ice_candidate_count"],
            json!(MAX_LOCAL_ICE_CANDIDATES)
        );
        assert_eq!(
            view["signaling"]["signaling_limits"]["local_ice_candidate_count"],
            json!(MAX_LOCAL_ICE_CANDIDATES)
        );
        assert_eq!(
            view["signaling"]["signaling_limits"]["ice_candidate_bytes"],
            json!(MAX_ICE_CANDIDATE_BYTES)
        );
        assert_eq!(
            view["signaling"]["signaling_limits"]["description_bytes"],
            json!(MAX_SIGNALING_DESCRIPTION_BYTES)
        );
        assert_eq!(
            view["signaling"]["remote_ice_candidates_elided"],
            json!(true)
        );
        assert_eq!(
            view["signaling"]["local_ice_candidates_truncated"],
            json!(false)
        );
        let serialized_len = serde_json::to_vec(&view).unwrap().len();
        let derived_bound = (MAX_SIGNALING_DESCRIPTION_BYTES * 2)
            + (MAX_LOCAL_ICE_CANDIDATES * MAX_ICE_CANDIDATE_BYTES * 3)
            + (256 * 1024);
        assert!(
            serialized_len <= derived_bound,
            "serialized session view grew past derived signaling bound: {serialized_len} > {derived_bound}"
        );
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

    #[test]
    fn direct_webrtc_transport_failure_suspends_session_for_a_new_generation() {
        let store = RemoteDesktopSessionStore::new();
        insert_test_session(&store, "rd-transport-failed");

        store.mark_direct_webrtc_generation_failed(
            "rd-transport-failed",
            TransportEpoch::new(1),
            "webrtc_peer_connection_failed",
            "device-side peer connection entered failed".to_string(),
        );

        store.with_sessions(|sessions| {
            let session = sessions.get_mut("rd-transport-failed").unwrap();
            let event = session
                .events()
                .into_iter()
                .find(|event| event["event_type"] == json!("TRANSPORT_FAILED"))
                .expect("transport failure event");
            assert_eq!(
                event["payload"]["reason"],
                json!("webrtc_peer_connection_failed")
            );
            assert_eq!(event["reason_code"], json!("transport_route_unavailable"));
            assert_eq!(event["recoverability"], json!("retry_session"));
            assert_eq!(
                event["payload"]["reason_code"],
                json!("transport_route_unavailable")
            );
            assert_eq!(event["payload"]["recoverability"], json!("retry_session"));
            assert_eq!(event["payload"]["failure_domain"], json!("transport"));
            assert_eq!(event["payload"]["frontend_action"], json!("retry_session"));
            assert_eq!(event["payload"]["transport_kind"], json!(TRANSPORT_WEBRTC));
            assert_eq!(event["payload"]["media_transport_ready"], json!(false));
            assert_eq!(event["payload"]["transport_epoch"], json!(1));
            assert!(!session.is_terminal());
            assert_eq!(serialize_session(session)["state"], json!("suspended"));
            assert!(session.terminal_receipt().is_none());
            assert!(session.subscribe_events().is_some());

            assert!(session.begin_webrtc_negotiation(TransportEpoch::new(2)));
            assert_eq!(session.transport_epoch(), Some(2));
            assert_eq!(serialize_session(session)["state"], json!("negotiating"));
            assert!(session.remote_ice_candidates().is_empty());
        });
    }

    #[test]
    fn transient_disconnect_degrades_and_recovers_without_replacing_the_epoch() {
        let store = RemoteDesktopSessionStore::new();
        insert_test_session(&store, "rd-transport-disconnected");
        let epoch = TransportEpoch::new(1);
        store.mark_direct_webrtc_media_ready("rd-transport-disconnected", epoch);
        store.with_sessions(|sessions| {
            assert!(sessions
                .get_mut("rd-transport-disconnected")
                .unwrap()
                .report_client_media_state(epoch, "presenting", None));
        });

        assert!(store.mark_direct_webrtc_disconnected("rd-transport-disconnected", epoch));
        store.with_sessions(|sessions| {
            let session = sessions.get_mut("rd-transport-disconnected").unwrap();
            assert!(!session.is_terminal());
            assert_eq!(session.transport_epoch(), Some(1));
            assert_eq!(serialize_session(session)["state"], json!("degraded"));
            assert!(session.report_client_media_state(epoch, "presenting", None));
            assert_eq!(serialize_session(session)["state"], json!("connected"));
        });
    }

    #[test]
    fn session_store_expires_target_rebind_deadline_for_bound_session() {
        let store = RemoteDesktopSessionStore::new();
        insert_test_session(&store, "rd-rebind-deadline");
        let inputs = store
            .target_observation_inputs_for_session("rd-rebind-deadline")
            .expect("target observation inputs");

        assert!(store
            .commit_target_observation_for_session(
                "rd-rebind-deadline",
                &inputs.binding_id,
                inputs.binding_epoch,
                TargetObservation::Lost {
                    reason: TargetResolutionError::TargetNotFound,
                    detail: "target disappeared".into(),
                    observed_at_ms: 100,
                },
            )
            .and_then(|commit| commit.media_source_lost)
            .is_none());
        store.commit_target_observation_for_session(
            "rd-rebind-deadline",
            &inputs.binding_id,
            inputs.binding_epoch,
            TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "target still disappeared".into(),
                observed_at_ms: 1_200,
            },
        );
        store.commit_target_observation_for_session(
            "rd-rebind-deadline",
            &inputs.binding_id,
            inputs.binding_epoch,
            TargetObservation::VisibilityChanged {
                visibility_state:
                    crate::daemon::plugins::remote_desktop::target_tracking::TargetVisibilityState::Visible,
                target_geometry_revision: 9,
                observed_at_ms: 1_300,
            },
        );

        assert!(store
            .expire_target_rebind_deadline_for_session(
                "rd-rebind-deadline",
                &inputs.binding_id,
                inputs.binding_epoch,
                31_299,
            )
            .is_none());
        let expiration = store
            .expire_target_rebind_deadline_for_session(
                "rd-rebind-deadline",
                &inputs.binding_id,
                inputs.binding_epoch,
                31_300,
            )
            .expect("store expires the bounded rebind attempt");
        assert!(expiration.into_media_source_lost().is_none());

        store.with_sessions(|sessions| {
            let session = sessions.get("rd-rebind-deadline").unwrap();
            let event = session
                .events()
                .into_iter()
                .find(|event| event["event_type"] == json!("TARGET_REBIND_FAILED"))
                .expect("deadline expiry event");
            assert_eq!(event["payload"]["detail"], json!("rebind_window_expired"));
            assert_eq!(event["payload"]["target_status"], json!("lost"));
            assert_eq!(event["payload"]["input_enabled"], json!(false));
        });
    }

    #[test]
    fn production_media_ready_requires_production_codec_and_sender_ready() {
        let store = RemoteDesktopSessionStore::new();
        insert_test_session(&store, "rd-non-production-ready");

        store.with_sessions(|sessions| {
            sessions
                .get_mut("rd-non-production-ready")
                .unwrap()
                .set_local_webrtc_answer(
                    TransportEpoch::new(1),
                    json!({"type": "answer", "sdp": "v=0\r\n"}),
                    "xcap-openh264-webrtc",
                    false,
                    "easynet:///r/acme/session/rd-non-production-ready/webrtc/1".to_string(),
                )
                .expect("non-production local answer records");
        });
        store.mark_direct_webrtc_media_ready("rd-non-production-ready", TransportEpoch::new(1));
        store.with_sessions(|sessions| {
            let view = serialize_session(sessions.get("rd-non-production-ready").unwrap());
            assert_eq!(view["media_transport_ready"], json!(true));
            assert_eq!(
                view["production_readiness"]["production_codec_negotiated"],
                json!(false)
            );
            assert_eq!(view["production_media_ready"], json!(false));
            assert_eq!(view["production_readiness"]["ready"], json!(false));
            assert_eq!(
                view["production_readiness"]["blocked_reason"],
                json!("production_codec_not_negotiated")
            );
            assert_eq!(view["transport"]["production_ready"], json!(false));
            assert_eq!(view["transports"][0]["production_ready"], json!(false));
            assert_eq!(
                view["transports"][0]["metadata"]["production_ready"],
                json!(false)
            );
        });

        insert_test_session(&store, "rd-production-ready");
        store.with_sessions(|sessions| {
            sessions
                .get_mut("rd-production-ready")
                .unwrap()
                .set_local_webrtc_answer(
                    TransportEpoch::new(1),
                    json!({"type": "answer", "sdp": "v=0\r\n"}),
                    "macos-sck-videotoolbox-webrtc",
                    true,
                    "easynet:///r/acme/session/rd-production-ready/webrtc/1".to_string(),
                )
                .expect("production local answer records");
            sessions
                .get_mut("rd-production-ready")
                .unwrap()
                .record_local_ice_candidate(json!({
                    "candidate": "candidate:host 1 UDP 2122252543 127.0.0.1 50000 typ host",
                    "sdpMid": "0",
                    "sdpMLineIndex": 0
                }))
                .expect("local host route candidate records");
        });
        store.mark_direct_webrtc_media_ready("rd-production-ready", TransportEpoch::new(1));
        store.with_sessions(|sessions| {
            let view = serialize_session(sessions.get("rd-production-ready").unwrap());
            assert_eq!(view["media_transport_ready"], json!(true));
            assert_eq!(
                view["production_readiness"]["production_codec_negotiated"],
                json!(true)
            );
            assert_eq!(
                view["production_readiness"]["client_media_ready"],
                json!(false)
            );
            assert_eq!(view["production_media_ready"], json!(false));
            assert_eq!(view["production_readiness"]["ready"], json!(false));
            assert_eq!(
                view["production_readiness"]["blocked_reason"],
                json!("client_media_not_presenting")
            );
            assert_eq!(view["transport"]["production_ready"], json!(false));
            assert_eq!(view["transports"][0]["production_ready"], json!(false));
        });
        store.with_sessions(|sessions| {
            assert!(sessions
                .get_mut("rd-production-ready")
                .unwrap()
                .report_client_media_state(TransportEpoch::new(1), "presenting", None));
        });
        store.with_sessions(|sessions| {
            let view = serialize_session(sessions.get("rd-production-ready").unwrap());
            assert_eq!(view["media_transport_ready"], json!(true));
            assert_eq!(
                view["production_readiness"]["production_codec_negotiated"],
                json!(true)
            );
            assert_eq!(
                view["production_readiness"]["client_media_ready"],
                json!(true)
            );
            assert_eq!(
                view["production_readiness"]["production_route_ready"],
                json!(false)
            );
            assert_eq!(view["production_media_ready"], json!(true));
            assert_eq!(view["production_readiness"]["ready"], json!(false));
            assert_eq!(
                view["production_readiness"]["blocked_reason"],
                json!("production_route_not_ready")
            );
            assert_eq!(
                view["production_readiness"]["route_readiness_blocker"]["reason_code"],
                json!("transport_route_unavailable")
            );
            assert_eq!(view["transport"]["production_ready"], json!(false));
            assert_eq!(view["transports"][0]["production_ready"], json!(false));
        });
        store.with_sessions(|sessions| {
            sessions
                .get_mut("rd-production-ready")
                .unwrap()
                .add_remote_ice_candidate(
                    json!({
                        "candidate": "candidate:relay 1 UDP 41819902 turn.example.test 3478 typ relay",
                        "relay_type": "turn",
                        "sdpMid": "0",
                        "sdpMLineIndex": 0
                    }),
                    "applied",
                    Some(TransportEpoch::new(1)),
                )
                .expect("relay route candidate records");
        });
        store.record_media_pipeline_stats(
            "rd-production-ready",
            TransportEpoch::new(1),
            json!({
                "audio_ready": true,
                "audio_media_observed": false,
                "audio_blocker": null,
            }),
        );
        store.with_sessions(|sessions| {
            let view = serialize_session(sessions.get("rd-production-ready").unwrap());
            assert_eq!(view["media_transport_ready"], json!(true));
            assert_eq!(
                view["production_readiness"]["production_codec_negotiated"],
                json!(true)
            );
            assert_eq!(
                view["production_readiness"]["client_media_ready"],
                json!(true)
            );
            assert_eq!(
                view["production_readiness"]["production_route_ready"],
                json!(true)
            );
            assert_eq!(view["production_media_ready"], json!(true));
            assert_eq!(view["production_readiness"]["ready"], json!(true));
            assert_eq!(view["production_readiness"]["blocked_reason"], json!(null));
            assert_eq!(view["transport"]["production_ready"], json!(true));
            assert_eq!(view["transports"][0]["production_ready"], json!(true));
            assert_eq!(
                view["transports"][0]["metadata"]["production_ready"],
                json!(true)
            );
        });
    }
}
