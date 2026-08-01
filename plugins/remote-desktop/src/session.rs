// EasyNet CLI — remote desktop session model
// ==========================================
//
// File: plugins/remote-desktop/src/session.rs
// Description: Session state and bounded event log for the remote desktop plugin.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tokio::sync::{broadcast, watch};

use crate::daemon::persistence::resources::ResourceType;
use crate::daemon::plugins::remote_desktop::constants::REASON_SESSION_EXPIRED;
use crate::daemon::plugins::remote_desktop::event_log::RemoteDesktopEventLog;
use crate::daemon::plugins::remote_desktop::request::{
    RemoteDesktopInputPolicy, RemoteDesktopVideoConstraints,
};
use crate::daemon::plugins::remote_desktop::session_events;
pub(in crate::daemon::plugins::remote_desktop) use crate::daemon::plugins::remote_desktop::session_identity::RemoteDesktopSessionInit;
use crate::daemon::plugins::remote_desktop::session_identity::RemoteDesktopSessionProfile;
use crate::daemon::plugins::remote_desktop::session_lease::RemoteDesktopLease;
use crate::daemon::plugins::remote_desktop::session_signaling::RemoteDesktopSignalingState;
use crate::daemon::plugins::remote_desktop::session_state::RemoteDesktopSessionStateMachine;
pub(in crate::daemon::plugins::remote_desktop) use crate::daemon::plugins::remote_desktop::session_state::RemoteDesktopState;
use crate::daemon::plugins::remote_desktop::session_transport_state::RemoteDesktopTransportState;

/// Runtime state for one remote desktop session.
///
/// Invariant 1: every lifecycle transition that changes `state`,
/// `media_transport_ready`, signaling diagnostics, `end_reason`, or the event
/// log goes through this type's methods.
///
/// Invariant 2: terminal sessions are stable. Methods that would mutate
/// active transport state return without changing terminal sessions.
///
/// Invariant 3: event ordering is delegated to the owned event log, so
/// callers cannot emit out-of-order lifecycle rows by hand.
#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopSession {
    profile: RemoteDesktopSessionProfile,
    lifecycle: RemoteDesktopSessionStateMachine,
    lease: RemoteDesktopLease,
    signaling: RemoteDesktopSignalingState,
    transport: RemoteDesktopTransportState,
    event_log: RemoteDesktopEventLog,
}

impl RemoteDesktopSession {
    /// Construct a negotiating session and emit the initial
    /// `SESSION_CREATED` event.
    pub(in crate::daemon::plugins::remote_desktop) fn new(init: RemoteDesktopSessionInit) -> Self {
        let now = now_ms();
        let (profile, lease_ttl_ms) = RemoteDesktopSessionProfile::from_init(init);
        let mut session = Self {
            profile,
            lifecycle: RemoteDesktopSessionStateMachine::new(),
            lease: RemoteDesktopLease::new(now, lease_ttl_ms),
            signaling: RemoteDesktopSignalingState::new(),
            transport: RemoteDesktopTransportState::new(),
            event_log: RemoteDesktopEventLog::new(),
        };
        session.push_projected_event(session_events::session_created());
        session
    }

    /// Stable opaque identifier for this remote desktop session.
    pub(in crate::daemon::plugins::remote_desktop) fn session_id(&self) -> &str {
        self.profile.session_id()
    }

    /// Return whether the caller supplied the session's bearer token.
    ///
    /// This intentionally avoids exposing token storage to mutation callers.
    pub(in crate::daemon::plugins::remote_desktop) fn matches_session_token(
        &self,
        token: &str,
    ) -> bool {
        self.profile.matches_session_token(token)
    }

    /// Opaque token returned only by create-session responses.
    pub(in crate::daemon::plugins::remote_desktop) fn session_token_for_create_response(
        &self,
    ) -> &str {
        self.profile.session_token_for_create_response()
    }

    /// Authenticated creator caller captured when the session was created.
    pub(in crate::daemon::plugins::remote_desktop) fn creator_caller_ura(&self) -> Option<&str> {
        self.profile.creator_caller_ura()
    }

    /// Immutable local-user consent grant captured at creation.
    pub(in crate::daemon::plugins::remote_desktop) fn consent(
        &self,
    ) -> &crate::daemon::plugins::remote_desktop::session_consent::RemoteDesktopConsentGrant {
        self.profile.consent()
    }

    /// Canonical resource URA that this session is allowed to operate on.
    pub(in crate::daemon::plugins::remote_desktop) fn subject_ura(&self) -> &str {
        self.profile.subject_ura()
    }

    /// Resource type captured at session creation.
    pub(in crate::daemon::plugins::remote_desktop) fn subject_type(&self) -> ResourceType {
        self.profile.subject_type()
    }

    /// Human-facing display name for the acted-on resource.
    pub(in crate::daemon::plugins::remote_desktop) fn subject_display_name(&self) -> &str {
        self.profile.subject_display_name()
    }

    /// Requested session mode.
    pub(in crate::daemon::plugins::remote_desktop) fn mode(&self) -> &str {
        self.profile.mode()
    }

    /// Current lifecycle state.
    pub(in crate::daemon::plugins::remote_desktop) fn state(&self) -> RemoteDesktopState {
        self.lifecycle.state()
    }

    /// Whether this session is terminal and must not mutate transport state.
    pub(in crate::daemon::plugins::remote_desktop) fn is_terminal(&self) -> bool {
        self.lifecycle.is_terminal()
    }

    /// Creation timestamp in Unix milliseconds.
    pub(in crate::daemon::plugins::remote_desktop) fn created_at_ms(&self) -> u64 {
        self.lease.created_at_ms()
    }

    /// Last mutation timestamp in Unix milliseconds.
    pub(in crate::daemon::plugins::remote_desktop) fn updated_at_ms(&self) -> u64 {
        self.lease.updated_at_ms()
    }

    /// Current lease deadline in Unix milliseconds.
    pub(in crate::daemon::plugins::remote_desktop) fn lease_expires_at_ms(&self) -> u64 {
        self.lease.expires_at_ms()
    }

    /// Whether the lease has elapsed at `now`.
    pub(in crate::daemon::plugins::remote_desktop) fn is_expired_at(&self, now: u64) -> bool {
        self.lease.is_expired_at(now)
    }

    /// Ordered transport preference list captured at session creation.
    pub(in crate::daemon::plugins::remote_desktop) fn transport_preferences(&self) -> &[String] {
        self.profile.transport_preferences()
    }

    /// Video constraints captured at session creation.
    pub(in crate::daemon::plugins::remote_desktop) fn video(
        &self,
    ) -> &RemoteDesktopVideoConstraints {
        self.profile.video()
    }

    /// Input policy captured at session creation.
    pub(in crate::daemon::plugins::remote_desktop) fn input_policy(
        &self,
    ) -> &RemoteDesktopInputPolicy {
        self.profile.input_policy()
    }

    /// Local SDP description, when the device-side endpoint has produced one.
    pub(in crate::daemon::plugins::remote_desktop) fn local_description(&self) -> Option<Value> {
        self.signaling.local_description()
    }

    /// Remote SDP description received from the caller.
    pub(in crate::daemon::plugins::remote_desktop) fn remote_description(&self) -> Option<Value> {
        self.signaling.remote_description()
    }

    /// Remote ICE candidates accepted for this session.
    pub(in crate::daemon::plugins::remote_desktop) fn remote_ice_candidates(&self) -> Vec<Value> {
        self.signaling.remote_ice_candidates()
    }

    /// Local ICE candidates produced by the device-side endpoint.
    pub(in crate::daemon::plugins::remote_desktop) fn local_ice_candidates(&self) -> Vec<Value> {
        self.signaling.local_ice_candidates()
    }

    /// Latest ICE state reported by the WebRTC backend.
    pub(in crate::daemon::plugins::remote_desktop) fn webrtc_ice_state(&self) -> Option<&str> {
        self.signaling.webrtc_ice_state()
    }

    /// Latest peer-connection state reported by the WebRTC backend.
    pub(in crate::daemon::plugins::remote_desktop) fn webrtc_peer_state(&self) -> Option<&str> {
        self.signaling.webrtc_peer_state()
    }

    /// Latest transport/backend error reason.
    pub(in crate::daemon::plugins::remote_desktop) fn webrtc_error(&self) -> Option<&str> {
        self.signaling.webrtc_error()
    }

    /// Whether the production media plane is ready.
    pub(in crate::daemon::plugins::remote_desktop) fn media_transport_ready(&self) -> bool {
        self.transport.media_transport_ready()
    }

    /// Negotiated media codec metadata.
    pub(in crate::daemon::plugins::remote_desktop) fn negotiated_codec(&self) -> Option<Value> {
        self.signaling.negotiated_codec()
    }

    /// Latest media pipeline statistics.
    pub(in crate::daemon::plugins::remote_desktop) fn media_stats(&self) -> Option<Value> {
        self.transport.media_stats()
    }

    /// Whether the diagnostic preview transport is attached.
    pub(in crate::daemon::plugins::remote_desktop) fn preview_attached(&self) -> bool {
        self.transport.preview_attached()
    }

    /// Bounded event-log projection.
    pub(in crate::daemon::plugins::remote_desktop) fn events(&self) -> Vec<Value> {
        self.event_log.events()
    }

    /// Subscribe to live session events without exposing the sender.
    pub(in crate::daemon::plugins::remote_desktop) fn subscribe_events(
        &self,
    ) -> broadcast::Receiver<Value> {
        self.event_log.subscribe()
    }

    /// Terminal reason for closed or failed sessions.
    pub(in crate::daemon::plugins::remote_desktop) fn end_reason(&self) -> Option<&str> {
        self.lifecycle.end_reason()
    }

    fn push_event(&mut self, event_type: &str, payload: Value) {
        self.event_log.push(
            self.profile.session_id(),
            self.lifecycle.state(),
            event_type,
            payload,
        );
    }

    fn push_projected_event(&mut self, event: (&'static str, Value)) {
        let (event_type, payload) = event;
        self.push_event(event_type, payload);
    }

    fn touch(&mut self) {
        self.lease.touch(now_ms());
    }

    /// Commit a local or remote SDP description after validation.
    pub(in crate::daemon::plugins::remote_desktop) fn set_description(
        &mut self,
        side: &str,
        description: Value,
    ) -> anyhow::Result<()> {
        if self.lifecycle.is_terminal() {
            return Ok(());
        }
        self.signaling.set_description(side, description)?;
        if self.signaling.has_description() {
            self.lifecycle.mark_negotiating();
        }
        self.touch();
        self.push_projected_event(session_events::description_set(
            side,
            self.transport.media_transport_ready(),
        ));
        Ok(())
    }

    /// Append a remote ICE candidate after access and argument validation.
    pub(in crate::daemon::plugins::remote_desktop) fn add_remote_ice_candidate(
        &mut self,
        candidate: Value,
        applied_to_live_endpoint: bool,
    ) {
        if self.lifecycle.is_terminal() {
            return;
        }
        let candidate_count = self.signaling.push_remote_ice_candidate(candidate);
        self.touch();
        self.push_projected_event(session_events::remote_ice_candidate_added(
            candidate_count,
            applied_to_live_endpoint,
            self.transport.media_transport_ready(),
        ));
    }

    /// Mark the diagnostic preview stream attached over InvokeBidi.
    pub(in crate::daemon::plugins::remote_desktop) fn attach_preview_transport(
        &mut self,
        stop_tx: watch::Sender<bool>,
    ) -> Option<watch::Sender<bool>> {
        if self.lifecycle.is_terminal() {
            return None;
        }
        let old_stop = self.transport.attach_preview_transport(stop_tx);
        self.lifecycle.mark_preview_connected();
        self.touch();
        self.push_projected_event(session_events::preview_transport_connected());
        old_stop
    }

    /// Refresh the lease and return the new expiry timestamp.
    pub(in crate::daemon::plugins::remote_desktop) fn refresh_lease(
        &mut self,
        now: u64,
        lease_ttl_ms: u64,
    ) -> u64 {
        let lease_expires_at_ms = self.lease.refresh(now, lease_ttl_ms);
        self.push_projected_event(session_events::lease_refreshed(lease_expires_at_ms));
        lease_expires_at_ms
    }

    /// Record a deterministic transport-blocked outcome without
    /// pretending the media plane is ready.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_transport_blocked(
        &mut self,
        reason: &str,
        required_backend: &str,
    ) {
        if self.lifecycle.is_terminal() {
            return;
        }
        self.signaling.set_webrtc_error(reason);
        self.transport.mark_media_pending();
        self.touch();
        self.push_projected_event(session_events::transport_blocked(reason, required_backend));
    }

    /// Commit a successfully-created WebRTC answer and negotiated codec
    /// metadata. The session remains not-ready until ICE/media confirms
    /// connectivity.
    pub(in crate::daemon::plugins::remote_desktop) fn set_local_webrtc_answer(
        &mut self,
        answer: Value,
        backend_id: &str,
        production_ready: bool,
        endpoint_ura: String,
    ) {
        if self.lifecycle.is_terminal() {
            return;
        }
        self.signaling
            .set_local_webrtc_answer(answer, backend_id, production_ready, endpoint_ura);
        self.transport.mark_media_pending();
        self.touch();
        self.push_projected_event(session_events::local_webrtc_answer_set(
            backend_id,
            production_ready,
        ));
    }

    /// Append a local ICE candidate produced by the device-side WebRTC endpoint.
    pub(in crate::daemon::plugins::remote_desktop) fn record_local_ice_candidate(
        &mut self,
        candidate: Value,
    ) {
        if self.lifecycle.is_terminal() {
            return;
        }
        let candidate_count = self.signaling.push_local_ice_candidate(candidate.clone());
        self.touch();
        self.push_projected_event(session_events::local_ice_candidate(
            candidate,
            candidate_count,
            self.transport.media_transport_ready(),
        ));
    }

    /// Record a WebRTC diagnostic event and any state/error fields carried by it.
    pub(in crate::daemon::plugins::remote_desktop) fn record_webrtc_diagnostic(
        &mut self,
        event_type: &str,
        error: Option<String>,
        payload: Value,
    ) {
        if self.lifecycle.is_terminal() {
            return;
        }
        self.signaling.record_webrtc_diagnostic(error, &payload);
        self.touch();
        let payload = session_events::webrtc_diagnostic(
            self.transport.media_transport_ready(),
            payload,
            self.signaling.webrtc_ice_state(),
            self.signaling.webrtc_error(),
        );
        self.push_event(event_type, payload);
    }

    /// Record diagnostic input-channel activity without changing transport readiness.
    pub(in crate::daemon::plugins::remote_desktop) fn record_input_channel_event(
        &mut self,
        event_type: &str,
        payload: Value,
    ) {
        if self.lifecycle.is_terminal() {
            return;
        }
        self.touch();
        let payload = session_events::input_channel_diagnostic(
            self.transport.media_transport_ready(),
            payload,
        );
        self.push_event(event_type, payload);
    }

    /// Store latest media stats and emit a bounded event-log row.
    #[cfg(target_os = "macos")]
    pub(in crate::daemon::plugins::remote_desktop) fn record_media_stats(&mut self, stats: Value) {
        if self.lifecycle.is_terminal() {
            return;
        }
        self.transport.record_media_stats(stats.clone());
        self.touch();
        self.push_projected_event(session_events::media_pipeline_stats(
            self.transport.media_transport_ready(),
            stats,
        ));
    }

    /// Detach preview transport state and return the stop signal to notify.
    pub(in crate::daemon::plugins::remote_desktop) fn detach_preview_transport(
        &mut self,
    ) -> Option<watch::Sender<bool>> {
        self.transport.detach_preview_transport()
    }

    /// Detach an InvokeBidi preview worker that reached a normal terminal path.
    pub(in crate::daemon::plugins::remote_desktop) fn detach_preview_transport_from_worker(
        &mut self,
        reason: &str,
    ) -> Option<watch::Sender<bool>> {
        if self.lifecycle.is_terminal() || !self.transport.preview_attached() {
            return None;
        }
        let stop_tx = self.transport.detach_preview_transport();
        self.lifecycle.mark_negotiating();
        self.touch();
        self.push_projected_event(session_events::preview_transport_detached(reason));
        stop_tx
    }

    /// Mark an InvokeBidi preview worker failure as a terminal session failure.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_preview_transport_failed(
        &mut self,
        reason: &str,
        message: String,
    ) -> Option<watch::Sender<bool>> {
        if !self.lifecycle.fail(reason) {
            return None;
        }
        let stop_tx = self.transport.detach_preview_transport();
        self.touch();
        self.push_projected_event(session_events::preview_transport_failed(reason, message));
        stop_tx
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn install_preview_transport_for_test(
        &mut self,
        stop_tx: watch::Sender<bool>,
    ) {
        self.transport.install_preview_transport_for_test(stop_tx);
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn set_lease_expires_at_for_test(
        &mut self,
        lease_expires_at_ms: u64,
    ) {
        self.lease.set_expires_at_for_test(lease_expires_at_ms);
    }

    /// Close an active session through the caller-requested terminal path.
    pub(in crate::daemon::plugins::remote_desktop) fn close(&mut self, reason: &str) {
        if !self.lifecycle.mark_closing() {
            return;
        }
        self.touch();
        self.push_projected_event(session_events::session_closing(reason));
        self.lifecycle.mark_closed(reason);
        self.touch();
        self.push_projected_event(session_events::session_closed(reason));
    }

    /// Mark the session closed because its lease elapsed.
    pub(in crate::daemon::plugins::remote_desktop) fn expire(&mut self, now: u64) {
        if !self.lifecycle.expire(REASON_SESSION_EXPIRED) {
            return;
        }
        self.lease.touch(now);
        self.push_projected_event(session_events::session_expired(
            REASON_SESSION_EXPIRED,
            self.lease.expires_at_ms(),
        ));
    }

    /// Mark the production WebRTC media plane ready.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_webrtc_media_ready(
        &mut self,
        endpoint_ura: String,
    ) {
        if self.lifecycle.is_terminal() || !self.transport.mark_media_ready() {
            return;
        }
        self.lifecycle.mark_connected();
        self.touch();
        self.push_projected_event(session_events::webrtc_connected(endpoint_ura));
    }

    /// Mark a non-terminal session failed and record the failure event.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_webrtc_failed(
        &mut self,
        reason: &str,
        message: String,
    ) {
        if !self.lifecycle.fail(reason) {
            return;
        }
        self.transport.mark_media_pending();
        self.signaling.set_webrtc_error(reason);
        self.touch();
        self.push_projected_event(session_events::webrtc_failed(reason, message));
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
