// EasyNet CLI — remote desktop session store
// ===========================================
//
// File: plugins/remote-desktop/src/session_store.rs
// Description: Synchronized session map and store-level transport projections.
//
// Protocol Responsibility:
// - None. This store owns plugin-local RemoteApp aggregate concurrency.
//
// Implementation Approach:
// - Serialize short aggregate mutations through one map mutex and serialize
//   blocking terminal durability only through weakly indexed per-session locks.
//
// Usage Contract:
// - External I/O must never run while `RemoteDesktopSessionStoreGuard` is held.
//
// Architectural Position:
// - Remote-desktop plugin session aggregate owner.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use serde_json::Value;

use crate::daemon::plugins::remote_desktop::constants::direct_webrtc_endpoint_ura;
use crate::daemon::plugins::remote_desktop::sdp::ice_candidate_text;
use crate::daemon::plugins::remote_desktop::session::{
    RemoteDesktopSession, TargetCoherenceToken, TargetMediaSourceLost,
    TargetRebindDeadlineExpiration,
};
use crate::daemon::plugins::remote_desktop::session_events::{
    webrtc_transport_failure_context, WebRtcFailureEventKind,
};
use crate::daemon::plugins::remote_desktop::session_recovery::RemoteDesktopRecoverySnapshot;
use crate::daemon::plugins::remote_desktop::session_transport_state::{
    ClientMediaFeedback, PreviewTransportEpoch, TransportEpoch,
};
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, ResolvedCaptureTargetProof, TargetResolutionError,
};
use crate::daemon::plugins::remote_desktop::target_tracking::{
    TargetObservation, TargetRebindAttemptToken, TargetTrackerSnapshot,
};

pub(in crate::daemon::plugins::remote_desktop) const MAX_TERMINAL_ROWS_PER_ACTIVE_SESSION: usize =
    4;

/// Capability minted while one exact input transport generation is active.
///
/// The capability authorizes only reducing host effects (key/button release),
/// but remains valid after the aggregate enters Closing/Terminal or its row is
/// pruned. This is required because a channel task may be cancelled after the
/// session transition and must still undo presses it already applied.
#[derive(Debug)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetSafetyReleasePermit {
    operation_lock: Arc<Mutex<()>>,
    session_id: String,
    transport_epoch: TransportEpoch,
}

impl TargetSafetyReleasePermit {
    pub(in crate::daemon::plugins::remote_desktop) fn operation_lock(&self) -> Arc<Mutex<()>> {
        self.operation_lock.clone()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn operation_guard(&self) -> MutexGuard<'_, ()> {
        debug_assert!(!self.session_id.is_empty());
        debug_assert!(self.transport_epoch.value() > 0);
        match self.operation_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    #[cfg(test)]
    fn identity(&self) -> (&str, TransportEpoch) {
        (&self.session_id, self.transport_epoch)
    }
}

pub(in crate::daemon::plugins::remote_desktop) const fn max_session_rows_for_active_limit(
    max_active_sessions: usize,
) -> Option<usize> {
    max_active_sessions.checked_mul(1 + MAX_TERMINAL_ROWS_PER_ACTIVE_SESSION)
}

pub(in crate::daemon::plugins::remote_desktop) struct TargetObservationInputs {
    pub(in crate::daemon::plugins::remote_desktop) binding: RemoteAppTargetBinding,
    pub(in crate::daemon::plugins::remote_desktop) snapshot: TargetTrackerSnapshot,
    pub(in crate::daemon::plugins::remote_desktop) binding_id: String,
    pub(in crate::daemon::plugins::remote_desktop) binding_epoch: u64,
    pub(in crate::daemon::plugins::remote_desktop) coherence_token: TargetCoherenceToken,
    pub(in crate::daemon::plugins::remote_desktop) rebind_attempt_token:
        Option<TargetRebindAttemptToken>,
}

pub(in crate::daemon::plugins::remote_desktop) struct PendingMediaRebindInputs {
    pub(in crate::daemon::plugins::remote_desktop) binding: RemoteAppTargetBinding,
    pub(in crate::daemon::plugins::remote_desktop) attempt_token: TargetRebindAttemptToken,
}

pub(in crate::daemon::plugins::remote_desktop) struct TargetObservationCommit {
    pub(in crate::daemon::plugins::remote_desktop) state_changed: bool,
    pub(in crate::daemon::plugins::remote_desktop) media_source_lost: Option<TargetMediaSourceLost>,
    /// True only when this observation entered the fail-closed, recoverable
    /// permission verification phase.
    pub(in crate::daemon::plugins::remote_desktop) permission_verification_started: bool,
    /// True only when this exact observation won the transition into Closing
    /// because host target permission was revoked. It is not a projection of
    /// the session's generic Closing state.
    pub(in crate::daemon::plugins::remote_desktop) permission_revocation_started: bool,
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
    terminal_commit_locks: Mutex<HashMap<String, Weak<Mutex<()>>>>,
    target_operation_locks: Mutex<HashMap<String, Weak<Mutex<()>>>>,
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

    /// Return the process-local commit mutex for one session terminal row.
    /// Weak indexing bounds the auxiliary map to live commit operations instead
    /// of retaining one lock for every historical terminal session.
    pub(in crate::daemon::plugins::remote_desktop) fn terminal_commit_lock(
        &self,
        session_id: &str,
    ) -> Arc<Mutex<()>> {
        let mut locks = match self.terminal_commit_locks.lock() {
            Ok(locks) => locks,
            Err(poisoned) => poisoned.into_inner(),
        };
        locks.retain(|_, lock| lock.strong_count() != 0);
        if let Some(lock) = locks.get(session_id).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(session_id.to_string(), Arc::downgrade(&lock));
        lock
    }

    /// Return the process-local target-operation lease for one session.
    ///
    /// Host input/focus effects and target lifecycle/rebind commits hold this
    /// exclusive lease from aggregate validation through their linearization
    /// point.
    /// The global session-map mutex is never held while waiting for this lock or
    /// while performing host I/O.
    pub(in crate::daemon::plugins::remote_desktop) fn target_operation_lock(
        &self,
        session_id: &str,
    ) -> Arc<Mutex<()>> {
        let mut locks = match self.target_operation_locks.lock() {
            Ok(locks) => locks,
            Err(poisoned) => poisoned.into_inner(),
        };
        locks.retain(|_, lock| lock.strong_count() != 0);
        if let Some(lock) = locks.get(session_id).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(session_id.to_string(), Arc::downgrade(&lock));
        lock
    }

    /// Mint the cancellation-safe reducing-operation capability for one exact
    /// direct WebRTC input generation.
    ///
    /// The operation gate closes the race with transport replacement while the
    /// permit is issued. The returned strong lock keeps the same gate alive
    /// even if the weak store index and terminal session row are later pruned.
    pub(in crate::daemon::plugins::remote_desktop) fn issue_target_safety_release_permit(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
    ) -> Option<TargetSafetyReleasePermit> {
        let operation_lock = self.target_operation_lock(session_id);
        let _operation = match operation_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut sessions = self.lock();
        let session = sessions.get_mut(session_id)?;
        if session.is_terminal()
            || session.is_terminating()
            || session.transport_epoch() != Some(epoch.value())
        {
            return None;
        }
        session.reserve_target_operation();
        drop(sessions);
        drop(_operation);
        Some(TargetSafetyReleasePermit {
            operation_lock,
            session_id: session_id.to_string(),
            transport_epoch: epoch,
        })
    }

    /// Execute one bounded session mutation while holding the same
    /// per-session gate used by target host effects.
    ///
    /// Transport generation replacement and retirement use this boundary so
    /// an admitted input/focus effect cannot cross into a different transport
    /// epoch. The closure must remain aggregate-local and must not perform
    /// host or network I/O.
    pub(in crate::daemon::plugins::remote_desktop) fn with_target_operation_session<R>(
        &self,
        session_id: &str,
        f: impl FnOnce(Option<&mut RemoteDesktopSession>) -> R,
    ) -> R {
        let operation_lock = self.target_operation_lock(session_id);
        let _operation = match operation_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut sessions = self.lock();
        f(sessions.get_mut(session_id))
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
        let operation_lock = self.target_operation_lock(session_id);
        let _operation = match operation_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
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
        let operation_lock = self.target_operation_lock(session_id);
        let _operation = match operation_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let sessions = self.lock();
        let session = sessions.get(session_id)?;
        if session.is_terminal() || session.is_terminating() {
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
            coherence_token: session.target_coherence_token(),
            rebind_attempt_token: session.target_rebind_attempt_token(),
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn commit_target_observation_for_session(
        &self,
        session_id: &str,
        binding_id: &str,
        binding_epoch: u64,
        expected_snapshot: &TargetTrackerSnapshot,
        expected_coherence: &TargetCoherenceToken,
        observation: TargetObservation,
    ) -> Option<TargetObservationCommit> {
        self.commit_target_observation_with_closing_checkpoint(
            session_id,
            binding_id,
            binding_epoch,
            expected_snapshot,
            expected_coherence,
            observation,
            |_| true,
        )
    }

    /// Commit one provider observation and, when it wins permission-revoked
    /// Closing, publish the exact recovery checkpoint before releasing the
    /// per-session operation gate.
    pub(in crate::daemon::plugins::remote_desktop) fn commit_target_observation_with_closing_checkpoint(
        &self,
        session_id: &str,
        binding_id: &str,
        binding_epoch: u64,
        expected_snapshot: &TargetTrackerSnapshot,
        expected_coherence: &TargetCoherenceToken,
        observation: TargetObservation,
        closing_checkpoint: impl FnOnce(&RemoteDesktopRecoverySnapshot) -> bool,
    ) -> Option<TargetObservationCommit> {
        let operation_lock = self.target_operation_lock(session_id);
        let _operation = match operation_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let permission_verification = matches!(
            observation,
            TargetObservation::PermissionVerificationRequired { .. }
        );
        let permission_revocation =
            matches!(observation, TargetObservation::PermissionRevoked { .. });
        let closing_intent = {
            let sessions = self.lock();
            let session = sessions.get(session_id)?;
            let binding = session.target_binding();
            if session.is_terminal()
                || session.is_terminating()
                || binding.binding_id() != binding_id
                || binding.binding_epoch() != binding_epoch
                || session.target_snapshot() != expected_snapshot
                || !session.target_coherence_matches(expected_coherence)
            {
                return None;
            }
            permission_revocation
                .then(|| {
                    RemoteDesktopRecoverySnapshot::prepare_closing_intent(
                        session,
                        crate::daemon::plugins::remote_desktop::constants::REASON_TARGET_PERMISSION_REVOKED,
                    )
                })
                .transpose()
        };
        let closing_intent = match closing_intent {
            Ok(closing_intent) => closing_intent,
            Err(error) => {
                eprintln!(
                    "[remote-desktop] failed to prepare permission-revocation Closing intent for {session_id}: {error}"
                );
                return None;
            }
        };
        // Persist the write-ahead intent before mutating the aggregate. The
        // operation gate remains held, so failure leaves a retryable Active
        // session and cannot race a host-visible effect.
        if closing_intent
            .as_ref()
            .is_some_and(|snapshot| !closing_checkpoint(snapshot))
        {
            return None;
        }
        let mut sessions = self.lock();
        let session = sessions.get_mut(session_id)?;
        let binding = session.target_binding();
        if session.is_terminal()
            || session.is_terminating()
            || binding.binding_id() != binding_id
            || binding.binding_epoch() != binding_epoch
            || session.target_snapshot() != expected_snapshot
            || !session.target_coherence_matches(expected_coherence)
        {
            return None;
        }
        session.reserve_target_operation();
        let previous_target_snapshot = session.target_snapshot().clone();
        let previous_sequence = session.latest_event_sequence();
        let media_source_lost = session.record_target_observation(observation);
        let permission_verification_started =
            permission_verification && session.target_snapshot().permission_verification_pending();
        let permission_revocation_started = permission_revocation && session.is_terminating();
        let commit = TargetObservationCommit {
            state_changed: session.target_snapshot() != &previous_target_snapshot
                || session.latest_event_sequence() != previous_sequence,
            media_source_lost,
            permission_verification_started,
            permission_revocation_started,
        };
        Some(commit)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn expire_target_rebind_deadline_for_session(
        &self,
        session_id: &str,
        binding_id: &str,
        binding_epoch: u64,
        expected_attempt: &TargetRebindAttemptToken,
        observed_at_ms: u64,
    ) -> Option<TargetRebindDeadlineExpiration> {
        let operation_lock = self.target_operation_lock(session_id);
        let _operation = match operation_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut sessions = self.lock();
        let session = sessions.get_mut(session_id)?;
        let binding = session.target_binding();
        if session.is_terminal()
            || session.is_terminating()
            || binding.binding_id() != binding_id
            || binding.binding_epoch() != binding_epoch
            || !session.matches_target_rebind_attempt(expected_attempt)
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
    ) -> Option<PendingMediaRebindInputs> {
        let sessions = self.lock();
        let session = sessions.get(session_id)?;
        if session.transport_epoch() != Some(epoch.value()) {
            return None;
        }
        let binding = session.pending_media_rebind_binding()?.clone();
        let attempt_token = session.target_rebind_attempt_token()?;
        (binding.media_source_epoch() > active_media_source_epoch).then_some(
            PendingMediaRebindInputs {
                binding,
                attempt_token,
            },
        )
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn commit_pending_media_rebind_for_session(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
        binding_epoch: u64,
        media_source_epoch: u64,
        expected_attempt: &TargetRebindAttemptToken,
        capture_proof: ResolvedCaptureTargetProof,
    ) -> bool {
        let operation_lock = self.target_operation_lock(session_id);
        let _operation = match operation_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        if !session.matches_target_rebind_attempt(expected_attempt) {
            return false;
        }
        session.reserve_target_operation();
        session.commit_pending_media_rebind(epoch, binding_epoch, media_source_epoch, capture_proof)
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn supersede_pending_media_rebind_for_session(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
        expected_attempt: &TargetRebindAttemptToken,
        reason: TargetResolutionError,
        detail: String,
    ) -> bool {
        let operation_lock = self.target_operation_lock(session_id);
        let _operation = match operation_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        if !session.matches_target_rebind_attempt(expected_attempt) {
            return false;
        }
        session.reserve_target_operation();
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
        self.with_target_operation_session(session_id, |session| {
            let Some(session) = session else {
                return false;
            };
            session.mark_webrtc_generation_failed_with_context(
                epoch, event_kind, reason, message, context,
            )
        })
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
        epoch: PreviewTransportEpoch,
        reason: &str,
    ) {
        self.with_target_operation_session(session_id, |session| {
            let Some(session) = session else {
                return;
            };
            let _ = session.detach_preview_transport_from_worker(epoch, reason);
        });
    }

    /// Mark the diagnostic InvokeBidi preview transport failed after capture
    /// or encoding terminates before a clean close.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_preview_transport_failed(
        &self,
        session_id: &str,
        epoch: PreviewTransportEpoch,
        reason: &str,
        message: String,
    ) {
        self.with_target_operation_session(session_id, |session| {
            let Some(session) = session else {
                return;
            };
            let _ = session.mark_preview_transport_failed(epoch, reason, message);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use serde_json::json;

    use crate::daemon::plugins::remote_desktop::constants::{
        MAX_ICE_CANDIDATE_BYTES, MAX_LOCAL_ICE_CANDIDATES, MAX_SIGNALING_DESCRIPTION_BYTES,
        TRANSPORT_WEBRTC,
    };
    use crate::daemon::plugins::remote_desktop::test_support::{
        admit_decoded_video_for_test, test_session_init,
    };
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
            assert!(session.begin_close("test_terminal"));
            session.finish_close("test_terminal");
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
    fn recovery_row_bound_includes_active_and_terminal_capacity() {
        assert_eq!(max_session_rows_for_active_limit(128), Some(640));
        assert_eq!(max_session_rows_for_active_limit(0), Some(0));
        assert_eq!(max_session_rows_for_active_limit(usize::MAX), None);
    }

    #[test]
    fn terminal_rows_are_removed_when_no_active_sessions_remain() {
        let store = RemoteDesktopSessionStore::new();
        let mut seeded_sessions = Vec::new();
        for index in 0..3 {
            let session_id = format!("terminal-only-{index}");
            let mut session = test_session(&session_id);
            assert!(session.begin_close("test_terminal"));
            session.finish_close("test_terminal");
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
    fn input_host_effect_gate_serializes_direct_epoch_replacement() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        insert_test_session(&store, "rd-epoch-replacement-gate");
        let operation_lock = store.target_operation_lock("rd-epoch-replacement-gate");
        let input_host_effect = operation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (started_tx, started_rx) = mpsc::channel();
        let (completed_tx, completed_rx) = mpsc::channel();
        let replacement_store = Arc::clone(&store);

        let replacement = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let advanced = replacement_store.with_target_operation_session(
                "rd-epoch-replacement-gate",
                |session| {
                    session
                        .expect("test session remains present")
                        .begin_webrtc_negotiation(TransportEpoch::new(2))
                },
            );
            completed_tx.send(advanced).unwrap();
        });

        started_rx.recv().unwrap();
        assert!(
            completed_rx
                .recv_timeout(Duration::from_millis(50))
                .is_err(),
            "epoch replacement must wait until the admitted host effect releases its gate"
        );
        store.with_sessions(|sessions| {
            assert_eq!(
                sessions
                    .get("rd-epoch-replacement-gate")
                    .unwrap()
                    .transport_epoch(),
                Some(1)
            );
        });

        drop(input_host_effect);
        assert!(completed_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        replacement.join().unwrap();
        store.with_sessions(|sessions| {
            assert_eq!(
                sessions
                    .get("rd-epoch-replacement-gate")
                    .unwrap()
                    .transport_epoch(),
                Some(2)
            );
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
        let mut inputs = store
            .target_observation_inputs_for_session("rd-rebind-deadline")
            .expect("target observation inputs");

        assert!(store
            .commit_target_observation_for_session(
                "rd-rebind-deadline",
                &inputs.binding_id,
                inputs.binding_epoch,
                &inputs.snapshot,
                &inputs.coherence_token,
                TargetObservation::Lost {
                    reason: TargetResolutionError::TargetNotFound,
                    detail: "target disappeared".into(),
                    observed_at_ms: 100,
                },
            )
            .and_then(|commit| commit.media_source_lost)
            .is_none());
        inputs = store
            .target_observation_inputs_for_session("rd-rebind-deadline")
            .expect("target observation inputs after first loss sample");
        store.commit_target_observation_for_session(
            "rd-rebind-deadline",
            &inputs.binding_id,
            inputs.binding_epoch,
            &inputs.snapshot,
            &inputs.coherence_token,
            TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "target still disappeared".into(),
                observed_at_ms: 1_200,
            },
        );
        inputs = store
            .target_observation_inputs_for_session("rd-rebind-deadline")
            .expect("target observation inputs after committed loss");
        store.commit_target_observation_for_session(
            "rd-rebind-deadline",
            &inputs.binding_id,
            inputs.binding_epoch,
            &inputs.snapshot,
            &inputs.coherence_token,
            TargetObservation::VisibilityChanged {
                visibility_state:
                    crate::daemon::plugins::remote_desktop::target_tracking::TargetVisibilityState::Visible,
                target_geometry_revision: 9,
                observed_at_ms: 1_300,
            },
        );
        inputs = store
            .target_observation_inputs_for_session("rd-rebind-deadline")
            .expect("target observation inputs after rebind start");
        let stale_attempt = inputs
            .rebind_attempt_token
            .clone()
            .expect("rebind attempt token");

        assert!(store
            .expire_target_rebind_deadline_for_session(
                "rd-rebind-deadline",
                &inputs.binding_id,
                inputs.binding_epoch,
                &stale_attempt,
                31_299,
            )
            .is_none());
        let expiration = store
            .expire_target_rebind_deadline_for_session(
                "rd-rebind-deadline",
                &inputs.binding_id,
                inputs.binding_epoch,
                &stale_attempt,
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
    fn target_observation_commit_rejects_a_stale_full_snapshot() {
        let store = RemoteDesktopSessionStore::new();
        insert_test_session(&store, "rd-target-snapshot-cas");
        let stale = store
            .target_observation_inputs_for_session("rd-target-snapshot-cas")
            .expect("initial target observation inputs");

        store.with_sessions(|sessions| {
            sessions
                .get_mut("rd-target-snapshot-cas")
                .expect("target session")
                .record_target_observation(TargetObservation::FocusChanged {
                    focused: false,
                    observed_at_ms: 10,
                });
        });

        assert!(store
            .commit_target_observation_for_session(
                "rd-target-snapshot-cas",
                &stale.binding_id,
                stale.binding_epoch,
                &stale.snapshot,
                &stale.coherence_token,
                TargetObservation::GeometryChanged {
                    geometry: crate::daemon::plugins::remote_desktop::target::TargetGeometry {
                        x: Some(10.0),
                        y: Some(20.0),
                        width: Some(300.0),
                        height: Some(200.0),
                    },
                    target_geometry_revision: 2,
                    observed_at_ms: 20,
                },
            )
            .is_none());
        store.with_sessions(|sessions| {
            let snapshot = sessions
                .get("rd-target-snapshot-cas")
                .expect("target session")
                .target_snapshot();
            assert_eq!(snapshot.focused(), Some(false));
            assert_ne!(snapshot.target_geometry_revision(), 2);
        });
    }

    #[test]
    fn target_operation_gate_excludes_a_concurrent_target_transition() {
        let store = RemoteDesktopSessionStore::new();
        let operation_lock = store.target_operation_lock("rd-target-operation-lock");
        let operation = operation_lock.lock().expect("target effect lease");
        assert!(operation_lock.try_lock().is_err());
        drop(operation);
        assert!(operation_lock.try_lock().is_ok());
    }

    #[test]
    fn safety_release_permit_is_transport_exact_and_survives_terminal_row_removal() {
        let store = RemoteDesktopSessionStore::new();
        let session_id = "rd-safety-release-permit";
        insert_test_session(&store, session_id);

        assert!(store
            .issue_target_safety_release_permit(session_id, TransportEpoch::new(2))
            .is_none());
        let permit = store
            .issue_target_safety_release_permit(session_id, TransportEpoch::new(1))
            .expect("active exact generation must mint reducing-operation permit");
        assert_eq!(permit.identity(), (session_id, TransportEpoch::new(1)));

        store.with_sessions(|sessions| {
            sessions.remove(session_id);
        });
        let operation_lock = permit.operation_lock();
        let operation = permit.operation_guard();
        assert!(operation_lock.try_lock().is_err());
        drop(operation);
        assert!(operation_lock.try_lock().is_ok());
    }

    #[test]
    fn host_operation_reservation_rejects_a_pre_effect_observation_token() {
        let store = RemoteDesktopSessionStore::new();
        insert_test_session(&store, "rd-target-operation-token");
        let stale = store
            .target_observation_inputs_for_session("rd-target-operation-token")
            .expect("target observation inputs");

        let operation_lock = store.target_operation_lock("rd-target-operation-token");
        let operation = operation_lock.lock().expect("target operation gate");
        store.with_sessions(|sessions| {
            sessions
                .get_mut("rd-target-operation-token")
                .expect("target session")
                .reserve_target_operation();
        });
        drop(operation);

        assert!(store
            .commit_target_observation_for_session(
                "rd-target-operation-token",
                &stale.binding_id,
                stale.binding_epoch,
                &stale.snapshot,
                &stale.coherence_token,
                TargetObservation::FocusChanged {
                    focused: false,
                    observed_at_ms: 20,
                },
            )
            .is_none());
    }

    #[test]
    fn accepted_noop_observation_still_fences_an_older_provider_sample() {
        let store = RemoteDesktopSessionStore::new();
        insert_test_session(&store, "rd-target-noop-token");
        let first = store
            .target_observation_inputs_for_session("rd-target-noop-token")
            .expect("first target observation inputs");
        let stale = store
            .target_observation_inputs_for_session("rd-target-noop-token")
            .expect("concurrent target observation inputs");

        assert!(store
            .commit_target_observation_for_session(
                "rd-target-noop-token",
                &first.binding_id,
                first.binding_epoch,
                &first.snapshot,
                &first.coherence_token,
                TargetObservation::VisibilityChanged {
                    visibility_state:
                        crate::daemon::plugins::remote_desktop::target_tracking::TargetVisibilityState::Visible,
                    target_geometry_revision: first.snapshot.target_geometry_revision(),
                    observed_at_ms: 20,
                },
            )
            .is_some());
        assert!(store
            .commit_target_observation_for_session(
                "rd-target-noop-token",
                &stale.binding_id,
                stale.binding_epoch,
                &stale.snapshot,
                &stale.coherence_token,
                TargetObservation::FocusChanged {
                    focused: false,
                    observed_at_ms: 30,
                },
            )
            .is_none());
    }

    #[test]
    fn permission_revocation_checkpoints_closing_before_releasing_operation_gate() {
        let store = RemoteDesktopSessionStore::new();
        let session_id = "rd-permission-checkpoint-gate";
        insert_test_session(&store, session_id);
        let inputs = store
            .target_observation_inputs_for_session(session_id)
            .expect("permission target observation inputs");
        let operation_lock = store.target_operation_lock(session_id);
        let mut checkpointed = false;

        let commit = store
            .commit_target_observation_with_closing_checkpoint(
                session_id,
                &inputs.binding_id,
                inputs.binding_epoch,
                &inputs.snapshot,
                &inputs.coherence_token,
                TargetObservation::PermissionRevoked {
                    detail: "screen recording permission revoked".to_string(),
                    observed_at_ms: 40,
                },
                |snapshot| {
                    assert_eq!(snapshot.lifecycle_state(), "closing");
                    assert!(
                        operation_lock.try_lock().is_err(),
                        "Closing checkpoint must execute before the operation gate is released"
                    );
                    checkpointed = true;
                    true
                },
            )
            .expect("permission observation commits");

        assert!(commit.permission_revocation_started);
        assert!(checkpointed);
        assert!(operation_lock.try_lock().is_ok());
    }

    #[test]
    fn permission_revocation_checkpoint_failure_preserves_retryable_active_session() {
        let store = RemoteDesktopSessionStore::new();
        let session_id = "rd-permission-checkpoint-failure";
        insert_test_session(&store, session_id);
        let inputs = store
            .target_observation_inputs_for_session(session_id)
            .expect("permission target observation inputs");

        let commit = store.commit_target_observation_with_closing_checkpoint(
            session_id,
            &inputs.binding_id,
            inputs.binding_epoch,
            &inputs.snapshot,
            &inputs.coherence_token,
            TargetObservation::PermissionRevoked {
                detail: "screen recording permission revoked".to_string(),
                observed_at_ms: 40,
            },
            |snapshot| {
                assert_eq!(snapshot.lifecycle_state(), "closing");
                false
            },
        );

        assert!(commit.is_none());
        store.with_sessions(|rows| {
            let session = rows.get(session_id).expect("session remains present");
            assert!(!session.is_terminating());
            assert!(!session.is_terminal());
            assert_ne!(session.state().json_name(), "closing");
        });
        assert!(store
            .target_observation_inputs_for_session(session_id)
            .is_some());
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
                    json!({"type": "answer", "sdp": "v=0\r\n", "media_scope": "video_only"}),
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
                json!(true)
            );
            assert_eq!(
                view["production_readiness"]["production_backend_ready"],
                json!(false)
            );
            assert_eq!(view["production_media_ready"], json!(false));
            assert_eq!(view["production_readiness"]["ready"], json!(false));
            assert_eq!(
                view["production_readiness"]["blocked_reason"],
                json!("production_backend_not_ready")
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
                    json!({"type": "answer", "sdp": "v=0\r\n", "media_scope": "video_only"}),
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
                view["production_readiness"]["production_backend_ready"],
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
            let session = sessions.get_mut("rd-production-ready").unwrap();
            assert!(session.report_client_media_state(TransportEpoch::new(1), "presenting", None));
            admit_decoded_video_for_test(
                session,
                TransportEpoch::new(1),
                "test-production-pipeline",
            );
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
                "media_pipeline_id": "test-production-pipeline",
                "backend_id": "test-production-pipeline",
                "video_codec": "h264",
                "video_transport": "webrtc",
                "audio_ready": true,
                "audio_operational_ready": true,
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
