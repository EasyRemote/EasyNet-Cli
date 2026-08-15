// EasyNet CLI — remote desktop session model
// ==========================================
//
// File: plugins/remote-desktop/src/session.rs
// Description: Session state and bounded event log for the remote desktop plugin.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::sync::{broadcast, watch};

use crate::daemon::persistence::resources::ResourceType;
use crate::daemon::plugins::remote_desktop::constants::REASON_SESSION_EXPIRED;
use crate::daemon::plugins::remote_desktop::event_log::{
    RemoteDesktopEventLog, RemoteDesktopEventReplay,
};
use crate::daemon::plugins::remote_desktop::request::{
    RemoteDesktopInputPolicy, RemoteDesktopVideoConstraints,
};
use crate::daemon::plugins::remote_desktop::session_events;
pub(in crate::daemon::plugins::remote_desktop) use crate::daemon::plugins::remote_desktop::session_identity::RemoteDesktopSessionInit;
use crate::daemon::plugins::remote_desktop::session_identity::RemoteDesktopSessionProfile;
use crate::daemon::plugins::remote_desktop::session_consent_state::{
    RemoteDesktopConsentPhase, RemoteDesktopConsentState,
};
use crate::daemon::plugins::remote_desktop::session_lease::RemoteDesktopLease;
use crate::daemon::plugins::remote_desktop::session_signaling::RemoteDesktopSignalingState;
use crate::daemon::plugins::remote_desktop::session_state::{
    InputActivationGate, RemoteDesktopSessionPhase, RemoteDesktopSessionStateMachine,
};
pub(in crate::daemon::plugins::remote_desktop) use crate::daemon::plugins::remote_desktop::session_state::RemoteDesktopState;
use crate::daemon::plugins::remote_desktop::session_transport_state::{
    PrimaryMediaPhase, RemoteDesktopTransportState, TransportEpoch,
};
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, TargetResolutionError,
};
use crate::daemon::plugins::remote_desktop::target_tracking::{
    RemoteAppTargetBindingStateMachine, TargetObservation, TargetTrackerSnapshot,
    TargetTrackingEvent, TargetVisibilityState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetMediaSourceLost {
    pub(in crate::daemon::plugins::remote_desktop) transport_epoch: TransportEpoch,
    pub(in crate::daemon::plugins::remote_desktop) reason: TargetResolutionError,
}

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
    consent: RemoteDesktopConsentState,
    lifecycle: RemoteDesktopSessionStateMachine,
    lease: RemoteDesktopLease,
    signaling: RemoteDesktopSignalingState,
    transport: RemoteDesktopTransportState,
    target: RemoteAppTargetBindingStateMachine,
    event_log: RemoteDesktopEventLog,
}

impl RemoteDesktopSession {
    /// Construct a negotiating session and emit the initial
    /// `SESSION_CREATED` event.
    pub(in crate::daemon::plugins::remote_desktop) fn new(init: RemoteDesktopSessionInit) -> Self {
        let now = now_ms();
        let (profile, consent_grant, target_binding, lease_ttl_ms) =
            RemoteDesktopSessionProfile::from_init(init);
        let consent_epoch = target_binding.consent_epoch();
        let mut session = Self {
            target: RemoteAppTargetBindingStateMachine::from_binding(target_binding),
            consent: RemoteDesktopConsentState::active(consent_grant, consent_epoch),
            profile,
            lifecycle: RemoteDesktopSessionStateMachine::new(),
            lease: RemoteDesktopLease::new(now, lease_ttl_ms),
            signaling: RemoteDesktopSignalingState::new(),
            transport: RemoteDesktopTransportState::new(),
            event_log: RemoteDesktopEventLog::new(),
        };
        session.push_projected_event(session_events::session_created());
        session.push_projected_event(session_events::capture_target_resolved(
            session.target.binding(),
        ));
        session.push_projected_event(session_events::target_bound(session.target.binding()));
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
    pub(in crate::daemon::plugins::remote_desktop) fn creator_caller_ura(&self) -> &str {
        self.profile.creator_caller_ura()
    }

    /// Immutable local-user consent grant captured at creation.
    pub(in crate::daemon::plugins::remote_desktop) fn consent(
        &self,
    ) -> &crate::daemon::plugins::remote_desktop::session_consent::RemoteDesktopConsentGrant {
        self.consent.grant()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn consent_state(
        &self,
    ) -> &RemoteDesktopConsentState {
        &self.consent
    }

    pub(in crate::daemon::plugins::remote_desktop) fn consent_phase(
        &self,
    ) -> RemoteDesktopConsentPhase {
        self.consent.phase()
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

    /// Resolved target binding that owns the session's capture/input/audit boundary.
    pub(in crate::daemon::plugins::remote_desktop) fn target_binding(
        &self,
    ) -> &RemoteAppTargetBinding {
        self.target.binding()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_snapshot(
        &self,
    ) -> &TargetTrackerSnapshot {
        self.target.snapshot()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_tracking_state(&self) -> Value {
        self.target.snapshot().to_value()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn latest_target_diagnostic(&self) -> Value {
        self.target.snapshot().latest_diagnostic()
    }

    /// Requested session mode.
    pub(in crate::daemon::plugins::remote_desktop) fn mode(&self) -> &str {
        self.profile.mode()
    }

    /// Current lifecycle state.
    pub(in crate::daemon::plugins::remote_desktop) fn state(&self) -> RemoteDesktopState {
        self.lifecycle.state()
    }

    /// Precise SPEC lifecycle phase. The existing `state` field remains the
    /// stable coarse product projection.
    pub(in crate::daemon::plugins::remote_desktop) fn lifecycle_phase(
        &self,
    ) -> RemoteDesktopSessionPhase {
        self.lifecycle.phase()
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

    /// Remote ICE candidates accepted for this session.
    pub(in crate::daemon::plugins::remote_desktop) fn remote_ice_candidates(&self) -> Vec<Value> {
        self.signaling.remote_ice_candidates()
    }

    /// Local ICE candidates produced by the device-side endpoint.
    pub(in crate::daemon::plugins::remote_desktop) fn local_ice_candidates(&self) -> Vec<Value> {
        self.signaling.local_ice_candidates()
    }

    /// Bounded WebRTC signaling projection for public session views.
    pub(in crate::daemon::plugins::remote_desktop) fn signaling_view(
        &self,
        route_state: Value,
    ) -> Value {
        self.signaling.to_bounded_view(route_state)
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

    /// Whether a production backend is negotiated and actively sending media.
    ///
    /// Diagnostic previews and non-production fallback media can make transport
    /// progress, but they must not be reported to the UI as production online.
    pub(in crate::daemon::plugins::remote_desktop) fn production_media_ready(&self) -> bool {
        self.target.binding().production_scope_ready()
            && self.signaling.production_codec_negotiated()
            && self.transport.media_transport_ready()
            && self.transport.client_media_ready()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_scope_ready(&self) -> bool {
        self.target.binding().production_scope_ready()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn production_codec_negotiated(&self) -> bool {
        self.signaling.production_codec_negotiated()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn client_media_ready(&self) -> bool {
        self.transport.client_media_ready()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn transport_epoch(&self) -> Option<u64> {
        self.transport.active_epoch().map(TransportEpoch::value)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn transport_state(&self) -> Value {
        self.transport.projection()
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

    /// Bounded replay projection for `remote_desktop.watch_events`.
    pub(in crate::daemon::plugins::remote_desktop) fn replay_events_from(
        &self,
        from_sequence: u64,
    ) -> RemoteDesktopEventReplay {
        self.event_log.replay_from(
            self.profile.session_id(),
            self.lifecycle.state(),
            from_sequence,
        )
    }

    /// Subscribe to live session events without exposing the sender.
    pub(in crate::daemon::plugins::remote_desktop) fn subscribe_events(
        &self,
    ) -> Option<broadcast::Receiver<Value>> {
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

    fn push_target_tracking_event(&mut self, event: TargetTrackingEvent) {
        let event_type = event.event_type();
        let mut payload = event.payload();
        payload["transport_epoch"] = self
            .transport
            .active_epoch()
            .map(|epoch| json!(epoch.value()))
            .unwrap_or(Value::Null);
        self.push_event(event_type, payload);
    }

    pub(in crate::daemon::plugins::remote_desktop) fn record_target_observation(
        &mut self,
        observation: TargetObservation,
    ) -> Option<TargetMediaSourceLost> {
        if self.lifecycle.is_terminal() {
            return None;
        }
        let target_loss_reason = match &observation {
            TargetObservation::Lost { reason, .. } => Some(*reason),
            TargetObservation::VisibilityChanged {
                visibility_state: TargetVisibilityState::Lost,
                ..
            } => Some(TargetResolutionError::TargetNotFound),
            TargetObservation::PermissionRevoked { .. } => {
                Some(TargetResolutionError::TargetPermissionMissing)
            }
            TargetObservation::DisplayTopologyChanged {
                selected_display_available: false,
                ..
            } => Some(TargetResolutionError::TargetDisplayUnavailable),
            _ => None,
        };
        let permission_revoked =
            matches!(&observation, TargetObservation::PermissionRevoked { .. });
        let input_was_enabled = self.target.snapshot().input_enabled();
        let Some(event) = self.target.commit_observation(observation) else {
            if input_was_enabled && !self.target.snapshot().input_enabled() {
                self.lifecycle.deactivate_input_for_target_block();
                self.touch();
            }
            return None;
        };
        let mut media_loss = None;
        if target_loss_reason.is_some() {
            if permission_revoked {
                self.consent.revoke();
            }
            self.lifecycle.suspend();
            if let (Some(reason), Some(epoch)) = (target_loss_reason, self.transport.active_epoch())
            {
                if self.transport.mark_media_source_lost(epoch) {
                    media_loss = Some(TargetMediaSourceLost {
                        transport_epoch: epoch,
                        reason,
                    });
                }
            }
        } else if event.event_type() == "TARGET_REBIND_ATTEMPTED" {
            self.lifecycle.begin_rebinding();
        } else if event.event_type() == "TARGET_REBIND_FAILED" {
            self.lifecycle.reject_rebinding();
        } else if !self.target.snapshot().input_enabled() {
            self.lifecycle.deactivate_input_for_target_block();
        }
        self.touch();
        self.push_target_tracking_event(event);
        if let Some(media_loss) = media_loss {
            self.push_projected_event(session_events::media_source_lost(
                self.target.binding(),
                media_loss.reason,
                media_loss.transport_epoch.value(),
            ));
        }
        media_loss
    }

    fn touch(&mut self) {
        self.lease.touch(now_ms());
    }

    fn reconcile_lifecycle(&mut self) {
        if self.lifecycle.is_terminal() {
            return;
        }
        match self.transport.primary_phase() {
            Some(PrimaryMediaPhase::ClientPresenting) => {
                self.lifecycle.activate_media();
            }
            Some(
                PrimaryMediaPhase::Degraded
                | PrimaryMediaPhase::MediaSourceLost
                | PrimaryMediaPhase::Failed,
            ) => {
                self.lifecycle.project_degraded();
            }
            Some(PrimaryMediaPhase::DeviceSending) => {
                self.lifecycle.activate_media_awaiting_client();
            }
            Some(PrimaryMediaPhase::Negotiating) => {
                self.lifecycle.start_media();
            }
            None if self.transport.preview_attached() => {
                self.lifecycle.project_preview_connected();
            }
            None => {
                self.lifecycle.project_waiting_for_media();
            }
        }
    }

    /// Promote the session to `InputActive` only after the direct input channel
    /// and its transport epoch have been proven to belong to the current
    /// production media generation. Input policy and platform permission checks
    /// are performed by the input plane before this aggregate boundary is
    /// called.
    pub(in crate::daemon::plugins::remote_desktop) fn activate_input_for_transport_epoch(
        &mut self,
        epoch: TransportEpoch,
    ) -> bool {
        if self.lifecycle.is_terminal() || self.transport_epoch() != Some(epoch.value()) {
            return false;
        }
        if !self.consent.permits_media_input() {
            return false;
        }
        if self.transport.primary_phase() != Some(PrimaryMediaPhase::ClientPresenting) {
            return false;
        }
        if !self.target.snapshot().input_enabled() {
            return false;
        }
        let changed = self.lifecycle.activate_input(InputActivationGate::Ready);
        if changed {
            self.touch();
        }
        changed
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
            self.lifecycle.start_media();
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
        application_state: &str,
        transport_epoch: Option<TransportEpoch>,
    ) -> anyhow::Result<()> {
        if self.lifecycle.is_terminal() {
            return Ok(());
        }
        let candidate_count = self.signaling.push_remote_ice_candidate(candidate)?;
        self.touch();
        self.push_projected_event(session_events::remote_ice_candidate_added(
            candidate_count,
            application_state,
            transport_epoch.map(TransportEpoch::value),
            self.transport.media_transport_ready(),
        ));
        Ok(())
    }

    /// Reserve capacity for one remote ICE candidate before transport side effects.
    pub(in crate::daemon::plugins::remote_desktop) fn reserve_remote_ice_candidate_slot(
        &mut self,
    ) -> anyhow::Result<bool> {
        if self.lifecycle.is_terminal() {
            return Ok(false);
        }
        self.signaling.reserve_remote_ice_candidate_slot()?;
        Ok(true)
    }

    /// Commit a previously reserved remote ICE candidate after transport apply.
    pub(in crate::daemon::plugins::remote_desktop) fn commit_reserved_remote_ice_candidate(
        &mut self,
        candidate: Value,
        application_state: &str,
        transport_epoch: Option<TransportEpoch>,
    ) -> anyhow::Result<()> {
        if self.lifecycle.is_terminal() {
            self.signaling.release_remote_ice_candidate_slot();
            return Ok(());
        }
        let candidate_count = self
            .signaling
            .commit_reserved_remote_ice_candidate(candidate)?;
        self.touch();
        self.push_projected_event(session_events::remote_ice_candidate_added(
            candidate_count,
            application_state,
            transport_epoch.map(TransportEpoch::value),
            self.transport.media_transport_ready(),
        ));
        Ok(())
    }

    /// Release remote ICE candidate capacity reserved for an uncommitted apply.
    pub(in crate::daemon::plugins::remote_desktop) fn release_remote_ice_candidate_slot(&mut self) {
        self.signaling.release_remote_ice_candidate_slot();
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
        self.reconcile_lifecycle();
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
        self.touch();
        self.push_projected_event(session_events::transport_blocked(reason, required_backend));
    }

    /// Commit a successfully-created WebRTC answer and negotiated codec
    /// metadata. The session remains not-ready until ICE/media confirms
    /// connectivity.
    pub(in crate::daemon::plugins::remote_desktop) fn set_local_webrtc_answer(
        &mut self,
        epoch: TransportEpoch,
        answer: Value,
        backend_id: &str,
        production_ready: bool,
        endpoint_ura: String,
    ) {
        if self.lifecycle.is_terminal() || !self.transport.accepts_epoch(epoch) {
            return;
        }
        self.signaling
            .set_local_webrtc_answer(answer, backend_id, production_ready, endpoint_ura);
        self.reconcile_lifecycle();
        self.touch();
        self.push_projected_event(session_events::local_webrtc_answer_set(
            backend_id,
            production_ready,
            epoch.value(),
        ));
    }

    pub(in crate::daemon::plugins::remote_desktop) fn begin_webrtc_negotiation(
        &mut self,
        epoch: TransportEpoch,
    ) {
        if self.lifecycle.is_terminal() {
            return;
        }
        self.transport.begin_primary(epoch);
        self.lifecycle.start_media();
        self.reconcile_lifecycle();
        self.touch();
    }

    /// Append a local ICE candidate produced by the device-side WebRTC endpoint.
    pub(in crate::daemon::plugins::remote_desktop) fn record_local_ice_candidate(
        &mut self,
        candidate: Value,
    ) -> anyhow::Result<()> {
        if self.lifecycle.is_terminal() {
            return Ok(());
        }
        let candidate_count = self.signaling.push_local_ice_candidate(candidate.clone())?;
        self.touch();
        self.push_projected_event(session_events::local_ice_candidate(
            candidate,
            candidate_count,
            self.transport.media_transport_ready(),
        ));
        Ok(())
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
    pub(in crate::daemon::plugins::remote_desktop) fn record_media_stats(
        &mut self,
        epoch: TransportEpoch,
        stats: Value,
    ) {
        if self.lifecycle.is_terminal() {
            return;
        }
        if !self.transport.record_media_stats(epoch, stats.clone()) {
            return;
        }
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
        self.reconcile_lifecycle();
        self.touch();
        self.push_projected_event(session_events::preview_transport_detached(reason));
        stop_tx
    }

    /// Record an InvokeBidi preview failure without failing production media.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_preview_transport_failed(
        &mut self,
        reason: &str,
        message: String,
    ) -> Option<watch::Sender<bool>> {
        if self.lifecycle.is_terminal() || !self.transport.preview_attached() {
            return None;
        }
        let stop_tx = self.transport.detach_preview_transport();
        self.reconcile_lifecycle();
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
        if !self.lifecycle.begin_termination() {
            return;
        }
        self.touch();
        self.push_projected_event(session_events::session_closing(reason));
        self.consent.expire();
        self.lifecycle.terminate_closed(reason);
        self.touch();
        self.push_projected_event(session_events::session_closed(reason));
        self.event_log.close();
    }

    /// Mark the session closed because its lease elapsed.
    pub(in crate::daemon::plugins::remote_desktop) fn expire(&mut self, now: u64) {
        if !self.lifecycle.expire(REASON_SESSION_EXPIRED) {
            return;
        }
        self.consent.expire();
        self.lease.touch(now);
        self.push_projected_event(session_events::session_expired(
            REASON_SESSION_EXPIRED,
            self.lease.expires_at_ms(),
        ));
        self.event_log.close();
    }

    /// Mark the active production endpoint as accepting encoded media.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_webrtc_media_sending(
        &mut self,
        epoch: TransportEpoch,
        endpoint_ura: String,
    ) {
        if self.lifecycle.is_terminal() || !self.transport.mark_device_sending(epoch) {
            return;
        }
        self.record_target_observation(TargetObservation::VisibilityChanged {
            visibility_state: TargetVisibilityState::Visible,
            target_geometry_revision: self.target.snapshot().target_geometry_revision(),
            observed_at_ms: now_ms(),
        });
        self.reconcile_lifecycle();
        self.touch();
        self.push_projected_event(session_events::webrtc_sender_ready(
            endpoint_ura,
            epoch.value(),
        ));
    }

    pub(in crate::daemon::plugins::remote_desktop) fn report_client_media_state(
        &mut self,
        epoch: TransportEpoch,
        state: &str,
    ) -> bool {
        if self.lifecycle.is_terminal() {
            return false;
        }
        let changed = match state {
            "presenting" => self.transport.mark_client_presenting(epoch),
            "stalled" | "detached" => self.transport.mark_client_stalled(epoch),
            _ => false,
        };
        if !changed {
            return false;
        }
        self.reconcile_lifecycle();
        self.touch();
        self.push_projected_event(session_events::client_media_state_changed(
            state,
            epoch.value(),
        ));
        true
    }

    /// Mark a non-terminal session failed and record the failure event.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_webrtc_failed_with_context(
        &mut self,
        epoch: TransportEpoch,
        event_kind: session_events::WebRtcFailureEventKind,
        reason: &str,
        message: String,
        context: serde_json::Value,
    ) -> Option<watch::Sender<bool>> {
        if !self.transport.accepts_epoch(epoch) {
            return None;
        }
        if !self.lifecycle.fail(reason) {
            return None;
        }
        self.transport.mark_failed(epoch);
        let preview_stop = self.transport.detach_preview_transport();
        self.signaling.set_webrtc_error(reason);
        self.touch();
        self.push_projected_event(session_events::webrtc_failed_with_context(
            event_kind,
            reason,
            message,
            epoch.value(),
            context,
        ));
        self.event_log.close();
        preview_stop
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::daemon::plugins::remote_desktop::constants::direct_webrtc_endpoint_ura;
    use crate::daemon::plugins::remote_desktop::session::{
        RemoteDesktopSession, RemoteDesktopState,
    };
    use crate::daemon::plugins::remote_desktop::session_consent_state::RemoteDesktopConsentPhase;
    use crate::daemon::plugins::remote_desktop::session_state::RemoteDesktopSessionPhase;
    use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
    use crate::daemon::plugins::remote_desktop::target::{TargetGeometry, TargetResolutionError};
    use crate::daemon::plugins::remote_desktop::target_tracking::{
        TargetObservation, TargetVisibilityState,
    };
    use crate::daemon::plugins::remote_desktop::test_support::test_session_init;
    use crate::daemon::plugins::remote_desktop::view::serialize_session;

    #[test]
    fn session_commits_target_observations_through_owned_tracker() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-target-tracker",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));
        assert_eq!(session.target_tracking_state()["status"], json!("resolved"));

        session.record_target_observation(TargetObservation::GeometryChanged {
            geometry: TargetGeometry {
                x: Some(40.0),
                y: Some(60.0),
                width: Some(900.0),
                height: Some(700.0),
            },
            target_geometry_revision: 2,
            observed_at_ms: 100,
        });

        let events = session.events();
        assert!(events
            .iter()
            .any(|event| event["event_type"] == json!("TARGET_RESIZED")));
        assert_eq!(
            session.target_tracking_state()["target_geometry_revision"],
            json!(2)
        );
        assert_eq!(
            session.latest_target_diagnostic()["detail"],
            json!("target_geometry_changed")
        );

        session.record_target_observation(TargetObservation::Lost {
            reason: TargetResolutionError::TargetNotFound,
            detail: "target disappeared".into(),
            observed_at_ms: 200,
        });

        assert_eq!(session.target_tracking_state()["status"], json!("resolved"));
        assert!(!session
            .events()
            .iter()
            .any(|event| event["event_type"] == json!("TARGET_LOST")));

        session.record_target_observation(TargetObservation::Lost {
            reason: TargetResolutionError::TargetNotFound,
            detail: "target still disappeared".into(),
            observed_at_ms: 300,
        });

        assert_eq!(session.target_tracking_state()["status"], json!("lost"));
        assert!(session
            .events()
            .iter()
            .any(|event| event["event_type"] == json!("TARGET_LOST")));
        assert_eq!(
            session.latest_target_diagnostic()["frontend_action"],
            json!("refresh_targets")
        );
    }

    #[test]
    fn target_tracking_events_include_active_transport_epoch_at_session_boundary() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-target-transport-epoch",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(23);
        session.begin_webrtc_negotiation(epoch);
        session.mark_webrtc_media_sending(
            epoch,
            direct_webrtc_endpoint_ura("rd-target-transport-epoch"),
        );

        session.record_target_observation(TargetObservation::GeometryChanged {
            geometry: TargetGeometry {
                x: Some(40.0),
                y: Some(60.0),
                width: Some(900.0),
                height: Some(700.0),
            },
            target_geometry_revision: 2,
            observed_at_ms: 100,
        });

        let events = session.events();
        let target_event = events
            .iter()
            .find(|event| event["event_type"] == json!("TARGET_RESIZED"))
            .expect("TARGET_RESIZED event");
        assert_eq!(target_event["transport_epoch"], json!(epoch.value()));
        assert_eq!(
            target_event["payload"]["transport_epoch"],
            json!(epoch.value())
        );
        assert_eq!(
            target_event["binding_id"],
            json!(session.target_binding().binding_id())
        );
        assert_eq!(
            target_event["target_geometry_revision"],
            json!(session.target_tracking_state()["target_geometry_revision"]
                .as_u64()
                .unwrap())
        );
    }

    #[test]
    fn target_lost_stops_active_media_source_without_transport_failure() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-target-media-lost",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(7);
        session.begin_webrtc_negotiation(epoch);
        session
            .mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura("rd-target-media-lost"));
        assert_eq!(
            session.transport_state()["primary"],
            json!("device_sending")
        );
        assert_eq!(session.transport_state()["device_sending"], json!(true));

        assert!(session
            .record_target_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "target disappeared".into(),
                observed_at_ms: 200,
            })
            .is_none());
        let media_loss = session
            .record_target_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "target still disappeared".into(),
                observed_at_ms: 300,
            })
            .expect("debounced target loss must request active media source stop");

        assert_eq!(media_loss.transport_epoch, epoch);
        assert_eq!(media_loss.reason, TargetResolutionError::TargetNotFound);
        assert_eq!(
            session.transport_state()["primary"],
            json!("media_source_lost")
        );
        assert_eq!(session.state(), RemoteDesktopState::Suspended);
        assert_eq!(session.transport_state()["device_sending"], json!(false));
        assert_eq!(
            session.target_tracking_state()["input_enabled"],
            json!(false)
        );

        let events = session.events();
        let target_lost_index = events
            .iter()
            .position(|event| event["event_type"] == json!("TARGET_LOST"))
            .expect("TARGET_LOST event");
        let media_source_lost_index = events
            .iter()
            .position(|event| event["event_type"] == json!("MEDIA_SOURCE_LOST"))
            .expect("MEDIA_SOURCE_LOST event");
        assert!(
            target_lost_index < media_source_lost_index,
            "target lifecycle event must precede media source stop projection"
        );
        assert_eq!(events[target_lost_index]["state"], json!("suspended"));
        assert_eq!(
            events[target_lost_index]["state_proto"],
            json!("REMOTE_DESKTOP_SESSION_STATE_SUSPENDED")
        );
        assert_eq!(
            events[target_lost_index]["payload"]["failure_domain"],
            json!("target")
        );
        assert_eq!(
            events[target_lost_index]["payload"]["frontend_action"],
            json!("refresh_targets")
        );
        assert_eq!(
            events[target_lost_index]["payload"]["target_status"],
            json!("lost")
        );
        assert_eq!(
            events[target_lost_index]["payload"]["input_enabled"],
            json!(false)
        );
        assert_eq!(
            events[target_lost_index]["transport_epoch"],
            json!(epoch.value())
        );
        assert_eq!(
            events[target_lost_index]["payload"]["transport_epoch"],
            json!(epoch.value())
        );
        assert_eq!(events[media_source_lost_index]["state"], json!("suspended"));
        assert_eq!(
            events[media_source_lost_index]["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            events[media_source_lost_index]["binding_id"],
            json!(session.target_binding().binding_id())
        );
        assert_eq!(
            events[media_source_lost_index]["binding_epoch"],
            json!(session.target_binding().binding_epoch())
        );
        assert_eq!(
            events[media_source_lost_index]["target_identity_epoch"],
            json!(session.target_binding().target_identity_epoch())
        );
        assert_eq!(
            events[media_source_lost_index]["media_source_epoch"],
            json!(session.target_binding().media_source_epoch())
        );
        assert_eq!(
            events[media_source_lost_index]["payload"]["frontend_action"],
            json!("refresh_targets")
        );
    }

    #[test]
    fn input_activation_requires_current_client_presenting_epoch() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-input-active-gate",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(11);

        assert!(!session.activate_input_for_transport_epoch(epoch));
        session.begin_webrtc_negotiation(epoch);
        session
            .mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura("rd-input-active-gate"));
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::MediaActive
        );
        assert!(
            !session.activate_input_for_transport_epoch(epoch),
            "input cannot activate while media is device-sending but not client-presenting"
        );

        assert!(session.report_client_media_state(epoch, "presenting"));
        assert!(session.activate_input_for_transport_epoch(epoch));
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::InputActive
        );
        assert!(
            !session.activate_input_for_transport_epoch(TransportEpoch::new(12)),
            "stale or unrelated transport epochs cannot activate input"
        );
    }

    #[test]
    fn input_activation_requires_target_input_enabled_snapshot() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-input-target-gate",
            "easynet:///r/acme/resource/window.test",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(13);

        session.begin_webrtc_negotiation(epoch);
        session
            .mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura("rd-input-target-gate"));
        assert!(session.report_client_media_state(epoch, "presenting"));
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::MediaActive
        );

        session.record_target_observation(TargetObservation::FocusChanged {
            focused: false,
            observed_at_ms: 100,
        });

        assert_eq!(
            session.target_tracking_state()["input_enabled"],
            json!(false)
        );
        assert!(
            !session.activate_input_for_transport_epoch(epoch),
            "input cannot activate when the committed target snapshot is not input-enabled"
        );
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::MediaActive
        );
    }

    #[test]
    fn target_focus_loss_deactivates_existing_input_without_failing_transport() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-input-focus-loss",
            "easynet:///r/acme/resource/window.test",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(19);

        session.begin_webrtc_negotiation(epoch);
        session.mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura("rd-input-focus-loss"));
        assert!(session.report_client_media_state(epoch, "presenting"));
        assert!(session.activate_input_for_transport_epoch(epoch));
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::InputActive
        );

        session.record_target_observation(TargetObservation::FocusChanged {
            focused: false,
            observed_at_ms: 200,
        });

        assert_eq!(
            session.target_tracking_state()["input_enabled"],
            json!(false)
        );
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::MediaActive
        );
        assert_eq!(session.state(), RemoteDesktopState::Connected);
        assert!(session.media_transport_ready());
        assert!(session.client_media_ready());
        assert_eq!(
            session.transport_state()["primary"],
            json!("client_presenting")
        );

        let blurred = session
            .events()
            .into_iter()
            .find(|event| event["event_type"] == json!("TARGET_BLURRED"))
            .expect("TARGET_BLURRED event");
        assert_eq!(blurred["state"], json!("connected"));
        assert_eq!(blurred["transport_epoch"], json!(epoch.value()));
        assert_eq!(blurred["payload"]["focused"], json!(false));
    }

    #[test]
    fn pending_target_loss_deactivates_input_before_media_loss_debounce() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-input-pending-loss",
            "easynet:///r/acme/resource/window.test",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(23);

        session.begin_webrtc_negotiation(epoch);
        session
            .mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura("rd-input-pending-loss"));
        assert!(session.report_client_media_state(epoch, "presenting"));
        assert!(session.activate_input_for_transport_epoch(epoch));
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::InputActive
        );

        let media_loss = session.record_target_observation(TargetObservation::Lost {
            reason: TargetResolutionError::TargetNotFound,
            detail: "single transient miss".into(),
            observed_at_ms: 200,
        });

        assert!(
            media_loss.is_none(),
            "first target miss remains debounced for media-source loss"
        );
        assert_eq!(session.state(), RemoteDesktopState::Connected);
        assert!(session.media_transport_ready());
        assert!(session.client_media_ready());
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::MediaActive
        );
        assert_eq!(session.target_tracking_state()["status"], json!("resolved"));
        assert_eq!(
            session.target_tracking_state()["input_enabled"],
            json!(false)
        );
        assert_eq!(
            session.target_tracking_state()["input_blocked_reason"],
            json!("target_loss_pending")
        );
        assert!(
            session
                .events()
                .into_iter()
                .all(|event| event["event_type"] != json!("TARGET_LOST")),
            "pending loss must not emit committed TARGET_LOST before debounce"
        );
    }

    #[test]
    fn consent_revocation_suspends_media_and_blocks_input_activation() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-consent-revoked-gate",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(17);

        assert_eq!(session.consent_phase(), RemoteDesktopConsentPhase::Active);
        session.begin_webrtc_negotiation(epoch);
        session.mark_webrtc_media_sending(
            epoch,
            direct_webrtc_endpoint_ura("rd-consent-revoked-gate"),
        );
        assert!(session.report_client_media_state(epoch, "presenting"));
        assert!(session.activate_input_for_transport_epoch(epoch));
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::InputActive
        );

        let media_lost = session
            .record_target_observation(TargetObservation::PermissionRevoked {
                detail: "local_user_revoked_consent".to_string(),
                observed_at_ms: 500,
            })
            .expect("permission revocation must return active media generation for endpoint stop");
        assert_eq!(media_lost.transport_epoch, epoch);
        assert_eq!(
            media_lost.reason,
            TargetResolutionError::TargetPermissionMissing
        );
        assert_eq!(session.consent_phase(), RemoteDesktopConsentPhase::Revoked);
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::Suspended
        );
        assert_eq!(session.state(), RemoteDesktopState::Suspended);
        assert!(!session.production_media_ready());
        assert!(
            !session.activate_input_for_transport_epoch(epoch),
            "revoked consent must prevent input from reactivating even with the same transport epoch"
        );

        let events = session.events();
        let permission_revoked_index = events
            .iter()
            .position(|event| event["event_type"] == json!("TARGET_PERMISSION_REVOKED"))
            .expect("TARGET_PERMISSION_REVOKED event");
        let media_source_lost_index = events
            .iter()
            .position(|event| event["event_type"] == json!("MEDIA_SOURCE_LOST"))
            .expect("MEDIA_SOURCE_LOST event");
        assert!(
            permission_revoked_index < media_source_lost_index,
            "consent/permission event must precede media-source stop projection"
        );
        assert_eq!(
            events[permission_revoked_index]["payload"]["reason_code"],
            json!("target_permission_missing")
        );
        assert_eq!(
            events[media_source_lost_index]["payload"]["transport_epoch"],
            json!(epoch.value())
        );
    }

    #[test]
    fn production_media_ready_requires_target_scope_ready() {
        let mut init = test_session_init(
            "rd-production-scope-gate",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        );
        init.target_binding = init.target_binding.with_scope_audit_for_test(false, true);
        let mut session = RemoteDesktopSession::new(init);
        let epoch = TransportEpoch::new(13);

        session.begin_webrtc_negotiation(epoch);
        session.set_local_webrtc_answer(
            epoch,
            json!({"type": "answer", "sdp": "v=0"}),
            "sck-native",
            true,
            "easynet:///r/acme/ability/remote-desktop.transport".into(),
        );
        session.mark_webrtc_media_sending(
            epoch,
            direct_webrtc_endpoint_ura("rd-production-scope-gate"),
        );

        assert!(session.production_codec_negotiated());
        assert!(session.media_transport_ready());
        assert!(!session.target_scope_ready());
        assert!(
            !session.production_media_ready(),
            "scope widening or display fallback must prevent production online"
        );

        let view = serialize_session(&session);
        assert_eq!(view["target_binding"]["scope_ready"], json!(false));
        assert_eq!(
            view["production_readiness"]["target_scope_ready"],
            json!(false)
        );
        assert_eq!(
            view["production_readiness"]["production_codec_negotiated"],
            json!(true)
        );
        assert_eq!(
            view["production_readiness"]["media_transport_ready"],
            json!(true)
        );
        assert_eq!(view["production_readiness"]["ready"], json!(false));
        assert_eq!(
            view["production_readiness"]["blocked_reason"],
            json!("target_scope_not_ready")
        );
        assert_eq!(view["production_media_ready"], json!(false));

        let events = session.events();
        let target_bound = events
            .iter()
            .find(|event| event["event_type"] == json!("TARGET_BOUND"))
            .expect("TARGET_BOUND event");
        assert_eq!(target_bound["payload"]["scope_ready"], json!(false));
        assert_eq!(
            target_bound["payload"]["display_fallback_used"],
            json!(true)
        );
    }

    #[test]
    fn target_loss_rejects_late_client_media_state_without_degrading_session() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-target-loss-late-client-state",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(9);
        session.begin_webrtc_negotiation(epoch);
        session.mark_webrtc_media_sending(
            epoch,
            direct_webrtc_endpoint_ura("rd-target-loss-late-client-state"),
        );

        session.record_target_observation(TargetObservation::Lost {
            reason: TargetResolutionError::TargetNotFound,
            detail: "target disappeared".into(),
            observed_at_ms: 200,
        });
        session
            .record_target_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "target still disappeared".into(),
                observed_at_ms: 300,
            })
            .expect("debounced target loss must stop the active media source");

        assert_eq!(session.state(), RemoteDesktopState::Suspended);
        assert_eq!(
            session.transport_state()["primary"],
            json!("media_source_lost")
        );
        assert_eq!(session.transport_state()["device_sending"], json!(false));

        assert!(!session.report_client_media_state(epoch, "stalled"));
        assert_eq!(session.state(), RemoteDesktopState::Suspended);
        assert_eq!(
            session.transport_state()["primary"],
            json!("media_source_lost")
        );
        assert_eq!(session.transport_state()["device_sending"], json!(false));
    }

    #[test]
    fn target_reappearance_after_loss_emits_explicit_rebind_failure() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-target-rebind-failed",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(11);
        session.begin_webrtc_negotiation(epoch);
        session.set_local_webrtc_answer(
            epoch,
            json!({"type": "answer", "sdp": "v=0"}),
            "sck-native",
            true,
            "easynet:///r/acme/ability/remote-desktop.transport".into(),
        );
        session.mark_webrtc_media_sending(
            epoch,
            direct_webrtc_endpoint_ura("rd-target-rebind-failed"),
        );
        assert!(session.report_client_media_state(epoch, "presenting"));
        assert!(session.production_media_ready());

        session.record_target_observation(TargetObservation::Lost {
            reason: TargetResolutionError::TargetNotFound,
            detail: "target disappeared".into(),
            observed_at_ms: 200,
        });
        session
            .record_target_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "target still disappeared".into(),
                observed_at_ms: 300,
            })
            .expect("debounced target loss must stop the active media source");
        assert_eq!(session.state(), RemoteDesktopState::Suspended);
        assert!(!session.production_media_ready());

        assert!(
            session
                .record_target_observation(TargetObservation::GeometryChanged {
                    geometry: TargetGeometry {
                        x: Some(240.0),
                        y: Some(260.0),
                        width: Some(1280.0),
                        height: Some(720.0),
                    },
                    target_geometry_revision: 9,
                    observed_at_ms: 400,
                })
                .is_none(),
            "rebinding stops no additional media source"
        );

        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::Rebinding
        );

        assert!(
            session
                .record_target_observation(TargetObservation::VisibilityChanged {
                    visibility_state: TargetVisibilityState::Visible,
                    target_geometry_revision: 10,
                    observed_at_ms: 500,
                })
                .is_none(),
            "rebind failure stops no already-stopped media source"
        );

        assert_eq!(session.state(), RemoteDesktopState::Suspended);
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::Suspended
        );
        assert!(!session.production_media_ready());
        assert_eq!(
            session.transport_state()["primary"],
            json!("media_source_lost")
        );
        assert_eq!(
            session.target_tracking_state()["input_enabled"],
            json!(false)
        );
        let events = session.events();
        let rebind_attempted = events
            .iter()
            .find(|event| event["event_type"] == json!("TARGET_REBIND_ATTEMPTED"))
            .expect("target reappearance emits rebind attempted");
        assert_eq!(
            rebind_attempted["payload"]["target_status"],
            json!("rebinding")
        );
        let rebind_failed = events
            .iter()
            .find(|event| event["event_type"] == json!("TARGET_REBIND_FAILED"))
            .expect("reappearance without explicit rebind policy must be typed");
        assert_eq!(rebind_failed["state"], json!("suspended"));
        assert_eq!(
            rebind_failed["event_type_proto"],
            json!("REMOTE_DESKTOP_EVENT_TARGET_CHANGED")
        );
        assert_eq!(
            rebind_failed["payload"]["reason_code"],
            json!("explicit_rebind_required")
        );
        assert_eq!(
            rebind_failed["payload"]["frontend_action"],
            json!("refresh_targets")
        );
        assert_eq!(rebind_failed["payload"]["target_status"], json!("lost"));
        assert_eq!(rebind_failed["payload"]["input_enabled"], json!(false));
    }
}
