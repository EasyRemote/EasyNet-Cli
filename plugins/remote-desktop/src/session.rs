// EasyNet CLI — remote desktop session model
// ==========================================
//
// File: plugins/remote-desktop/src/session.rs
// Description: Session state and bounded event log for the remote desktop plugin.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::sync::{broadcast, watch};

use crate::daemon::persistence::resources::ResourceType;
use crate::daemon::plugins::remote_desktop::constants::{
    REASON_SESSION_EXPIRED, REASON_TARGET_PERMISSION_REVOKED,
};
use crate::daemon::plugins::remote_desktop::event_log::{
    RemoteDesktopEventLog, RemoteDesktopEventReplay,
};
use crate::daemon::plugins::remote_desktop::input::RemoteDesktopInputPolicy;
use crate::daemon::plugins::remote_desktop::request::RemoteDesktopVideoConstraints;
use crate::daemon::plugins::remote_desktop::relay_lease::{
    RemoteDesktopRelayLease, RemoteDesktopRelayLeaseAvailability,
};
use crate::daemon::plugins::remote_desktop::session_consent::RemoteDesktopConsentGrant;
use crate::daemon::plugins::remote_desktop::session_events;
use crate::daemon::plugins::remote_desktop::session_signaling::RemoteDesktopNegotiatedMediaScope;
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
use crate::daemon::plugins::remote_desktop::session_recovery::RemoteDesktopRecoverySnapshot;
pub(in crate::daemon::plugins::remote_desktop) use crate::daemon::plugins::remote_desktop::session_state::RemoteDesktopState;
use crate::daemon::plugins::remote_desktop::session_transport_state::{
    ClientMediaFeedback, ClientRenderEvidence, PreviewTransportEpoch, PrimaryMediaPhase,
    RemoteDesktopTransportState, TransportEpoch,
};
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, ResolvedCaptureTargetProof, TargetResolutionError,
};
use crate::daemon::plugins::remote_desktop::target_tracking::{
    RemoteAppTargetBindingStateMachine, TargetObservation, TargetTrackerSnapshot,
    TargetRebindAttemptToken, TargetTrackingEmission, TargetVisibilityState,
};

const CLIENT_RENDER_EVIDENCE_MAX_AGE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetMediaSourceLost {
    pub(in crate::daemon::plugins::remote_desktop) transport_epoch: TransportEpoch,
    pub(in crate::daemon::plugins::remote_desktop) reason: TargetResolutionError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetRebindDeadlineExpiration {
    media_source_lost: Option<TargetMediaSourceLost>,
}

/// Process-local CAS token for target observations performed outside the
/// session operation mutex. A fresh incarnation prevents session-id reuse from
/// accepting work sampled from an earlier aggregate row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetCoherenceToken {
    session_incarnation: String,
    revision: u64,
}

impl TargetRebindDeadlineExpiration {
    fn new(media_source_lost: Option<TargetMediaSourceLost>) -> Self {
        Self { media_source_lost }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn into_media_source_lost(
        self,
    ) -> Option<TargetMediaSourceLost> {
        self.media_source_lost
    }
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
    input_runtime_block_reason: Option<String>,
    terminal_receipt: Option<Value>,
    session_incarnation: String,
    target_coherence_revision: u64,
    relay_lease: RemoteDesktopRelayLeaseAvailability,
    /// Operational fence held only while this exact Closing revision is being
    /// promoted to the authoritative recovery slot. It is process-local and is
    /// intentionally absent from the recovery contract.
    terminal_commit_in_progress: bool,
}

#[derive(Debug)]
pub(in crate::daemon) enum RemoteDesktopRelayLeaseRotation {
    Installed,
    AlreadyOwned,
    Unowned(RemoteDesktopRelayLease),
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
            input_runtime_block_reason: None,
            terminal_receipt: None,
            session_incarnation: uuid::Uuid::new_v4().simple().to_string(),
            target_coherence_revision: 1,
            relay_lease: RemoteDesktopRelayLeaseAvailability::unavailable("hub_relay_not_acquired"),
            terminal_commit_in_progress: false,
        };
        session.push_projected_event(session_events::session_created());
        session.push_projected_event(session_events::capture_target_resolved(
            session.target.binding(),
        ));
        session.push_projected_event(session_events::target_bound(session.target.binding()));
        session
    }

    pub(in crate::daemon::plugins::remote_desktop) fn rehydrate(
        snapshot: &RemoteDesktopRecoverySnapshot,
    ) -> anyhow::Result<Self> {
        let target_binding = RemoteAppTargetBinding::from_recovery_value(
            snapshot.target_binding(),
            snapshot.subject_display_name(),
        )?;
        if target_binding.subject_ura() != snapshot.selected_resource_ura() {
            anyhow::bail!(
                "RemoteApp recovery snapshot subject does not match target binding subject"
            );
        }
        let consent_grant = RemoteDesktopConsentGrant::from_recovery_value(snapshot.consent())?;
        let consent_epoch = target_binding.consent_epoch();
        let (profile, _, target_binding, _) =
            RemoteDesktopSessionProfile::from_init(RemoteDesktopSessionInit {
                session_id: snapshot.session_id().to_string(),
                session_token: snapshot.session_token().to_string(),
                creator_caller_ura: snapshot.creator_caller_ura().to_string(),
                consent: consent_grant.clone(),
                target_binding,
                mode: snapshot.mode().to_string(),
                lease_ttl_ms: 1,
                transport_preferences: snapshot.transport_preferences().to_vec(),
                video: RemoteDesktopVideoConstraints::from_value(snapshot.video())?,
                input_policy: RemoteDesktopInputPolicy::from_value(snapshot.input_policy())?,
            });
        let terminal_receipt = snapshot.terminal_receipt();
        let terminal =
            terminal_receipt.is_some() || matches!(snapshot.lifecycle_state(), "closed" | "failed");
        let lifecycle = if terminal {
            let public_state = match snapshot.lifecycle_state() {
                "failed" => RemoteDesktopState::Failed,
                _ => RemoteDesktopState::Closed,
            };
            RemoteDesktopSessionStateMachine::rehydrate_terminal(
                public_state,
                recovery_terminal_reason(terminal_receipt.as_ref()),
            )?
        } else if snapshot.lifecycle_state() == "closing" {
            RemoteDesktopSessionStateMachine::rehydrate_terminating(
                snapshot
                    .termination_reason()
                    .ok_or_else(|| {
                        anyhow::anyhow!("Closing recovery snapshot is missing termination_reason")
                    })?
                    .to_string(),
            )?
        } else {
            RemoteDesktopSessionStateMachine::rehydrate_degraded()
        };
        let consent =
            RemoteDesktopConsentState::rehydrate(consent_grant.clone(), snapshot.consent())
                .unwrap_or_else(|_| {
                    RemoteDesktopConsentState::active(consent_grant, consent_epoch)
                });
        let mut session = Self {
            target: RemoteAppTargetBindingStateMachine::rehydrate(
                target_binding,
                snapshot.target_tracking(),
            )?,
            consent,
            profile,
            lifecycle,
            lease: RemoteDesktopLease::rehydrate(
                snapshot.created_at_ms(),
                snapshot.updated_at_ms(),
                snapshot.lease_expires_at_ms(),
            )?,
            signaling: RemoteDesktopSignalingState::new(),
            transport: RemoteDesktopTransportState::rehydrate(
                snapshot.transport_epoch_high_watermark(),
            ),
            event_log: RemoteDesktopEventLog::rehydrate(snapshot.events(), terminal)?,
            input_runtime_block_reason: if terminal {
                None
            } else {
                snapshot
                    .input_runtime_block_reason()
                    .map(ToString::to_string)
            },
            terminal_receipt,
            session_incarnation: uuid::Uuid::new_v4().simple().to_string(),
            target_coherence_revision: 1,
            relay_lease: RemoteDesktopRelayLeaseAvailability::unavailable(
                "hub_relay_requires_reacquire_after_recovery",
            ),
            terminal_commit_in_progress: false,
        };
        if !terminal {
            session.push_projected_event(session_events::session_rehydrated(
                session.target.binding(),
                snapshot.lifecycle_state(),
            ));
        }
        Ok(session)
    }

    /// Stable opaque identifier for this remote desktop session.
    pub(in crate::daemon::plugins::remote_desktop) fn session_id(&self) -> &str {
        self.profile.session_id()
    }

    pub(in crate::daemon) fn install_relay_lease(
        &mut self,
        relay_lease: RemoteDesktopRelayLeaseAvailability,
    ) {
        self.relay_lease = relay_lease;
    }

    pub(in crate::daemon) fn active_relay_lease(&self) -> Option<&RemoteDesktopRelayLease> {
        self.relay_lease.active()
    }

    /// Atomically rotate the Hub relay authorization owned by this session.
    ///
    /// Hub acquisition has already superseded the prior lease ID on success,
    /// so only the fresh authorization remains releasable. An idempotent
    /// duplicate refresh is reported as already owned and must not release the
    /// session's current lease. A genuinely rejected fresh lease is returned
    /// to the caller for release. This makes the current Hub authorization
    /// have one explicit owner across refresh/session-terminal races.
    pub(in crate::daemon) fn rotate_relay_lease_if_current(
        &mut self,
        expected_lease_id: &str,
        refreshed: RemoteDesktopRelayLease,
    ) -> RemoteDesktopRelayLeaseRotation {
        if self
            .active_relay_lease()
            .is_some_and(|lease| lease.lease_id() == refreshed.lease_id())
        {
            return RemoteDesktopRelayLeaseRotation::AlreadyOwned;
        }
        if self.is_terminal()
            || self
                .active_relay_lease()
                .is_none_or(|lease| lease.lease_id() != expected_lease_id)
        {
            return RemoteDesktopRelayLeaseRotation::Unowned(refreshed);
        }
        self.relay_lease = RemoteDesktopRelayLeaseAvailability::Active(refreshed);
        RemoteDesktopRelayLeaseRotation::Installed
    }

    pub(in crate::daemon) fn retire_relay_lease_if_current(
        &mut self,
        expected_lease_id: &str,
        reason: impl Into<String>,
    ) -> Option<RemoteDesktopRelayLease> {
        if self
            .active_relay_lease()
            .is_none_or(|lease| lease.lease_id() != expected_lease_id)
        {
            return None;
        }
        self.retire_relay_lease(reason)
    }

    pub(in crate::daemon) fn relay_lease_evidence(&self) -> Value {
        self.relay_lease.evidence_value()
    }

    pub(in crate::daemon) fn retire_relay_lease(
        &mut self,
        reason: impl Into<String>,
    ) -> Option<RemoteDesktopRelayLease> {
        let prior = std::mem::replace(
            &mut self.relay_lease,
            RemoteDesktopRelayLeaseAvailability::unavailable(reason),
        );
        match prior {
            RemoteDesktopRelayLeaseAvailability::Active(lease) => Some(lease),
            RemoteDesktopRelayLeaseAvailability::Unavailable { .. } => None,
        }
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

    pub(in crate::daemon::plugins::remote_desktop) fn session_token_for_recovery_snapshot(
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

    /// Resource type projected from the committed target binding.
    ///
    /// `subject_type` remains a public response projection for existing
    /// consumers, but it is not stored in the immutable profile. The committed
    /// `RemoteAppTargetBinding` is the only target-kind source of truth.
    pub(in crate::daemon::plugins::remote_desktop) fn subject_type(&self) -> ResourceType {
        self.target.binding().target_kind().resource_type()
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

    pub(in crate::daemon::plugins::remote_desktop) fn target_coherence_token(
        &self,
    ) -> TargetCoherenceToken {
        TargetCoherenceToken {
            session_incarnation: self.session_incarnation.clone(),
            revision: self.target_coherence_revision,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_coherence_matches(
        &self,
        expected: &TargetCoherenceToken,
    ) -> bool {
        self.session_incarnation == expected.session_incarnation
            && self.target_coherence_revision == expected.revision
    }

    /// Reserve the next target-affecting operation generation before a host
    /// effect starts. Failed or no-op host calls intentionally retain the new
    /// generation so observations sampled before the attempt cannot commit.
    pub(in crate::daemon::plugins::remote_desktop) fn reserve_target_operation(
        &mut self,
    ) -> TargetCoherenceToken {
        if let Some(next) = self.target_coherence_revision.checked_add(1) {
            self.target_coherence_revision = next;
        } else {
            // Rotate the process-local incarnation instead of allowing a
            // saturated revision to make distinct operations compare equal.
            self.session_incarnation = uuid::Uuid::new_v4().simple().to_string();
            self.target_coherence_revision = 1;
        }
        self.target_coherence_token()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_tracking_state(&self) -> Value {
        self.target.snapshot().to_value()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_tracking_recovery_value(
        &self,
    ) -> Value {
        self.target.snapshot().to_recovery_value()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn latest_target_diagnostic(&self) -> Value {
        self.target.snapshot().latest_diagnostic()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn input_runtime_block_reason(
        &self,
    ) -> Option<&str> {
        self.input_runtime_block_reason.as_deref()
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
            && self.signaling.production_backend_ready()
            && self.transport.media_transport_ready()
            && self.transport.client_media_ready()
            && self.client_decode_ready()
            && (!self
                .signaling
                .negotiated_media_scope()
                .is_some_and(|scope| scope.requires_audio())
                || self.audio_operational_ready())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_scope_ready(&self) -> bool {
        self.target.binding().production_scope_ready()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn production_codec_negotiated(&self) -> bool {
        self.signaling.production_codec_negotiated()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn production_backend_ready(&self) -> bool {
        self.signaling.production_backend_ready()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn client_media_ready(&self) -> bool {
        self.transport.client_media_ready()
    }

    /// Whether fresh daemon-admitted receiver evidence proves the browser
    /// decoded the exact current transport/target/media-source generation.
    pub(in crate::daemon::plugins::remote_desktop) fn client_decode_ready(&self) -> bool {
        let Some(epoch) = self.transport.active_epoch() else {
            return false;
        };
        let Some(evidence) = self.transport.client_render_evidence(epoch) else {
            return false;
        };
        self.client_render_evidence_matches(&evidence, Instant::now())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn audio_operational_ready(&self) -> bool {
        self.transport
            .media_stats()
            .as_ref()
            .and_then(|stats| stats.get("audio_operational_ready"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    fn client_render_evidence_matches(
        &self,
        evidence: &ClientRenderEvidence,
        now: Instant,
    ) -> bool {
        let Some(stats) = self.transport.media_stats() else {
            return false;
        };
        let Some(epoch) = self.transport.active_epoch() else {
            return false;
        };
        let binding = self.target.binding();
        let expected_pipeline = stats
            .get("media_pipeline_id")
            .or_else(|| stats.get("backend_id"))
            .and_then(Value::as_str);
        let expected_video_codec = stats.get("video_codec").and_then(Value::as_str);
        let expected_video_transport = stats.get("video_transport").and_then(Value::as_str);
        let expected_audio_codec = stats.get("audio_codec").and_then(Value::as_str);
        let audio_required = self
            .signaling
            .negotiated_media_scope()
            .is_some_and(|scope| scope.requires_audio());

        now.saturating_duration_since(evidence.received_at) <= CLIENT_RENDER_EVIDENCE_MAX_AGE
            && evidence.session_id == self.session_id()
            && evidence.selected_resource_ura == binding.subject_ura()
            && evidence.transport_epoch == epoch.value()
            && evidence.binding_id == binding.binding_id()
            && evidence.binding_epoch == binding.binding_epoch()
            && evidence.media_source_epoch == binding.media_source_epoch()
            && expected_pipeline == Some(evidence.media_pipeline_id.as_str())
            && expected_video_codec
                .is_some_and(|codec| codec.eq_ignore_ascii_case(&evidence.video_codec))
            && expected_video_transport
                .is_some_and(|transport| transport.eq_ignore_ascii_case(&evidence.video_transport))
            && evidence.decoded_video_frames > 0
            && evidence.frame_width > 0
            && evidence.frame_height > 0
            && (!audio_required
                || (expected_audio_codec.is_some_and(|codec| {
                    evidence
                        .audio_codec
                        .as_deref()
                        .is_some_and(|actual| codec.eq_ignore_ascii_case(actual))
                }) && (evidence.decoded_audio_packets > 0
                    || evidence.decoded_audio_samples > 0)))
    }

    pub(in crate::daemon::plugins::remote_desktop) fn transport_epoch(&self) -> Option<u64> {
        self.transport.active_epoch().map(TransportEpoch::value)
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn transport_epoch_high_watermark(
        &self,
    ) -> u64 {
        self.transport.epoch_high_watermark()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn transport_state(&self) -> Value {
        self.transport.projection()
    }

    /// Negotiated media codec metadata.
    pub(in crate::daemon::plugins::remote_desktop) fn negotiated_codec(&self) -> Option<Value> {
        self.signaling.negotiated_codec()
    }

    /// Media tracks negotiated for the active WebRTC transport generation.
    pub(in crate::daemon::plugins::remote_desktop) fn negotiated_media_scope(
        &self,
    ) -> Option<RemoteDesktopNegotiatedMediaScope> {
        self.signaling.negotiated_media_scope()
    }

    /// Latest media pipeline statistics.
    pub(in crate::daemon::plugins::remote_desktop) fn media_stats(&self) -> Option<Value> {
        self.transport.media_stats()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn client_media_feedback(
        &self,
        epoch: TransportEpoch,
    ) -> Option<ClientMediaFeedback> {
        self.transport.client_media_feedback(epoch)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn client_render_evidence(
        &self,
        epoch: TransportEpoch,
    ) -> Option<ClientRenderEvidence> {
        self.transport.client_render_evidence(epoch)
    }

    /// Whether the diagnostic preview transport is attached.
    pub(in crate::daemon::plugins::remote_desktop) fn preview_attached(&self) -> bool {
        self.transport.preview_attached()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn preview_transport_epoch(
        &self,
    ) -> Option<PreviewTransportEpoch> {
        self.transport.preview_epoch()
    }

    /// Bounded event-log projection.
    pub(in crate::daemon::plugins::remote_desktop) fn events(&self) -> Vec<Value> {
        self.event_log.events()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn latest_event_sequence(&self) -> u64 {
        self.event_log.latest_sequence()
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

    pub(in crate::daemon::plugins::remote_desktop) fn terminal_receipt(&self) -> Option<Value> {
        self.terminal_receipt.clone()
    }

    fn push_event(&mut self, event_type: &str, payload: Value) -> Value {
        self.event_log.push(
            self.profile.session_id(),
            self.lifecycle.state(),
            event_type,
            payload,
        )
    }

    fn push_projected_event(
        &mut self,
        event: session_events::RemoteDesktopEventProjection,
    ) -> Value {
        let event = event.with_target_binding_context(self.target.binding());
        self.push_event(event.event_type(), event.into_payload())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn record_target_monitor_worker_crashed(
        &mut self,
        failed_generation: u64,
        detail: &str,
    ) {
        self.push_projected_event(session_events::target_monitor_worker_crashed(
            failed_generation,
            detail,
        ));
    }

    pub(in crate::daemon::plugins::remote_desktop) fn record_target_monitor_worker_restarted(
        &mut self,
        failed_generation: u64,
        restarted_generation: u64,
    ) {
        self.push_projected_event(session_events::target_monitor_worker_restarted(
            failed_generation,
            restarted_generation,
        ));
    }

    pub(in crate::daemon::plugins::remote_desktop) fn record_target_monitor_restarted(
        &mut self,
        failed_generation: u64,
        restarted_generation: u64,
    ) {
        self.push_projected_event(session_events::target_monitor_restarted(
            failed_generation,
            restarted_generation,
        ));
    }

    fn push_target_tracking_event(&mut self, event: TargetTrackingEmission) {
        let transport_epoch = self
            .transport
            .active_epoch()
            .map(|epoch| json!(epoch.value()))
            .unwrap_or(Value::Null);
        for (event_type, mut payload) in event.ordered_events() {
            payload["transport_epoch"] = transport_epoch.clone();
            self.push_event(event_type, payload);
        }
    }

    fn project_terminal_receipt(&self, reason: &str, terminal_event: &Value) -> Value {
        json!({
            "receipt_type": "remoteapp.session.terminal.v1",
            "session_id": self.session_id(),
            "subject_ura": self.subject_ura(),
            "subject_type": self.subject_type().as_str(),
            "binding_id": self.target.binding().binding_id(),
            "binding_epoch": self.target.binding().binding_epoch(),
            "target_identity_epoch": self.target.binding().target_identity_epoch(),
            "target_geometry_revision": self.target.binding().target_geometry_revision(),
            "media_source_epoch": self.target.binding().media_source_epoch(),
            "consent_epoch": self.target.binding().consent_epoch(),
            "reason": reason,
            "reason_code": reason,
            "terminal_event_id": terminal_event.get("event_id").cloned().unwrap_or(Value::Null),
            "terminal_event_sequence": terminal_event.get("sequence").cloned().unwrap_or(Value::Null),
            "terminal_event_type": terminal_event.get("event_type").cloned().unwrap_or(Value::Null),
            "closed_at_ms": terminal_event.get("at_ms").cloned().unwrap_or(Value::Null),
            "state": terminal_event.get("state").cloned().unwrap_or(Value::Null),
            "state_proto": terminal_event.get("state_proto").cloned().unwrap_or(Value::Null),
            "lifecycle_phase": self.lifecycle.phase().as_str(),
            "terminal": true,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn record_target_observation(
        &mut self,
        observation: TargetObservation,
    ) -> Option<TargetMediaSourceLost> {
        if self.lifecycle.is_terminal() || self.lifecycle.is_terminating() {
            return None;
        }
        let target_loss_reason = match &observation {
            TargetObservation::Lost { reason, .. } => Some(*reason),
            TargetObservation::MonitorUnavailable { .. } => {
                Some(TargetResolutionError::CaptureBackendUnavailable)
            }
            TargetObservation::VisibilityChanged {
                visibility_state: TargetVisibilityState::Lost,
                ..
            } => Some(TargetResolutionError::TargetNotFound),
            TargetObservation::PermissionRevoked { .. } => {
                Some(TargetResolutionError::TargetPermissionMissing)
            }
            TargetObservation::PermissionVerificationRequired { .. } => {
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
        let media_source_active = self.transport.active_epoch().is_some();
        let Some(event) = self
            .target
            .commit_observation_with_media_source_activity(observation, media_source_active)
        else {
            if input_was_enabled && !self.target.snapshot().input_enabled() {
                self.lifecycle.deactivate_input_for_target_block();
                self.touch();
                if let Some(reason) = target_loss_reason {
                    self.push_projected_event(session_events::target_loss_pending(reason));
                }
            }
            return None;
        };
        let mut media_source_lost = None;
        if let Some(reason) = target_loss_reason {
            if permission_revoked {
                self.consent.revoke();
            }
            self.lifecycle.suspend();
            media_source_lost = self.mark_active_media_source_lost(reason);
        } else if event.event_type() == "TARGET_REBIND_ATTEMPTED" {
            self.lifecycle.begin_rebinding();
        } else if event.event_type() == "TARGET_REBIND_FAILED" {
            self.lifecycle.reject_rebinding();
        } else if !self.target.snapshot().input_enabled() {
            self.lifecycle.deactivate_input_for_target_block();
        }
        self.touch();
        self.push_target_tracking_event(event);
        self.push_media_source_lost_event(media_source_lost);
        if permission_revoked {
            self.begin_close_after_permission_revoked();
        }
        media_source_lost
    }

    pub(in crate::daemon::plugins::remote_desktop) fn record_authorized_target_focus(
        &mut self,
        observed_at_ms: u64,
        platform_backend: &str,
    ) -> (u64, u64) {
        let previous_target_focus_epoch = self.target.snapshot().target_focus_epoch();
        if self.product_operations_closed() {
            return (previous_target_focus_epoch, previous_target_focus_epoch);
        }
        if self.target.snapshot().focused() != Some(true) {
            self.record_target_observation(TargetObservation::FocusChanged {
                focused: true,
                observed_at_ms,
            });
        }
        let target_focus_epoch = self.target.snapshot().target_focus_epoch();
        self.push_projected_event(session_events::target_focus_applied(
            self.target.binding(),
            previous_target_focus_epoch,
            target_focus_epoch,
            observed_at_ms,
            platform_backend,
        ));
        self.touch();
        (previous_target_focus_epoch, target_focus_epoch)
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn pending_media_rebind_binding(
        &self,
    ) -> Option<&RemoteAppTargetBinding> {
        if self.product_operations_closed() {
            return None;
        }
        self.target.pending_media_rebind_binding()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_rebind_attempt_token(
        &self,
    ) -> Option<TargetRebindAttemptToken> {
        self.target.rebind_attempt_token()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn matches_target_rebind_attempt(
        &self,
        expected: &TargetRebindAttemptToken,
    ) -> bool {
        self.target.matches_rebind_attempt(expected)
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn commit_pending_media_rebind(
        &mut self,
        epoch: TransportEpoch,
        binding_epoch: u64,
        media_source_epoch: u64,
        capture_proof: ResolvedCaptureTargetProof,
    ) -> bool {
        if self.product_operations_closed() || self.transport.active_epoch() != Some(epoch) {
            return false;
        }
        let Some(event) = self.target.commit_pending_media_rebind(
            binding_epoch,
            media_source_epoch,
            capture_proof,
            now_ms(),
        ) else {
            return false;
        };
        self.lifecycle.complete_rebinding();
        self.touch();
        self.push_target_tracking_event(event);
        true
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn supersede_pending_media_rebind(
        &mut self,
        epoch: TransportEpoch,
        reason: TargetResolutionError,
        detail: String,
    ) -> bool {
        if self.product_operations_closed() || self.transport.active_epoch() != Some(epoch) {
            return false;
        }
        let Some(event) = self
            .target
            .supersede_pending_media_rebind(reason, detail, now_ms())
        else {
            return false;
        };
        self.lifecycle.reject_rebinding();
        self.lifecycle.start_media();
        self.reconcile_lifecycle();
        self.touch();
        self.push_target_tracking_event(event);
        true
    }

    pub(in crate::daemon::plugins::remote_desktop) fn expire_target_rebind_deadline(
        &mut self,
        observed_at_ms: u64,
    ) -> Option<TargetRebindDeadlineExpiration> {
        if self.product_operations_closed() {
            return None;
        }
        let event = self.target.expire_rebind_deadline(observed_at_ms)?;
        self.lifecycle.reject_rebinding();
        let media_source_lost =
            self.mark_active_media_source_lost(TargetResolutionError::TargetStale);
        self.touch();
        self.push_target_tracking_event(event);
        self.push_media_source_lost_event(media_source_lost);
        Some(TargetRebindDeadlineExpiration::new(media_source_lost))
    }

    fn mark_active_media_source_lost(
        &mut self,
        reason: TargetResolutionError,
    ) -> Option<TargetMediaSourceLost> {
        let epoch = self.transport.active_epoch()?;
        self.transport
            .mark_media_source_lost(epoch)
            .then_some(TargetMediaSourceLost {
                transport_epoch: epoch,
                reason,
            })
    }

    fn push_media_source_lost_event(&mut self, media_source_lost: Option<TargetMediaSourceLost>) {
        let Some(media_source_lost) = media_source_lost else {
            return;
        };
        self.push_projected_event(session_events::media_source_lost(
            self.target.binding(),
            media_source_lost.reason,
            media_source_lost.transport_epoch.value(),
        ));
    }

    fn touch(&mut self) {
        self.lease.touch(now_ms());
    }

    fn product_operations_closed(&self) -> bool {
        self.terminal_commit_in_progress
            || self.lifecycle.is_terminal()
            || self.lifecycle.is_terminating()
    }

    fn reconcile_lifecycle(&mut self) {
        if self.product_operations_closed() {
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
        if self.product_operations_closed() || self.transport_epoch() != Some(epoch.value()) {
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
            self.input_runtime_block_reason = None;
            self.touch();
        }
        changed
    }

    /// Confirm that an input frame has been accepted by the host OS for the
    /// current direct-input epoch. This is the recovery edge after a runtime
    /// permission block: the input plane supplies execution proof, while the
    /// aggregate owns lifecycle reactivation and blocker clearing.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_input_frame_applied(
        &mut self,
        epoch: TransportEpoch,
    ) -> bool {
        let had_runtime_block = self.input_runtime_block_reason.is_some();
        let changed = self.activate_input_for_transport_epoch(epoch);
        if had_runtime_block && changed {
            self.push_projected_event(session_events::input_permission_restored(
                self.target.binding(),
                epoch.value(),
                self.transport.media_transport_ready(),
            ));
        }
        changed
    }

    pub(in crate::daemon::plugins::remote_desktop) fn block_input_for_runtime_permission(
        &mut self,
        epoch: TransportEpoch,
        reason: &str,
    ) -> bool {
        if self.product_operations_closed() || self.transport_epoch() != Some(epoch.value()) {
            return false;
        }
        let changed = self.lifecycle.deactivate_input_for_runtime_block();
        if !changed {
            return false;
        }
        self.input_runtime_block_reason = Some(reason.to_string());
        self.touch();
        self.push_projected_event(session_events::input_permission_blocked(
            self.target.binding(),
            epoch.value(),
            reason,
            self.transport.media_transport_ready(),
        ));
        true
    }

    /// Commit a local or remote SDP description after validation.
    pub(in crate::daemon::plugins::remote_desktop) fn set_description(
        &mut self,
        side: &str,
        description: Value,
    ) -> anyhow::Result<()> {
        if self.product_operations_closed() {
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
        if self.product_operations_closed() {
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
        if self.product_operations_closed() {
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
        if self.product_operations_closed() {
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
        if self.terminal_commit_in_progress {
            return;
        }
        self.signaling.release_remote_ice_candidate_slot();
    }

    /// Mark the diagnostic preview stream attached over InvokeBidi.
    pub(in crate::daemon::plugins::remote_desktop) fn attach_preview_transport(
        &mut self,
        stop_tx: watch::Sender<bool>,
    ) -> Option<(PreviewTransportEpoch, Option<watch::Sender<bool>>)> {
        if self.product_operations_closed() {
            return None;
        }
        let attachment = self.transport.attach_preview_transport(stop_tx);
        self.reconcile_lifecycle();
        self.touch();
        self.push_projected_event(session_events::preview_transport_connected());
        Some(attachment)
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
        if self.product_operations_closed() {
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
    ) -> anyhow::Result<()> {
        if self.product_operations_closed() || !self.transport.accepts_epoch(epoch) {
            return Ok(());
        }
        self.signaling.set_local_webrtc_answer(
            answer,
            backend_id,
            production_ready,
            endpoint_ura,
        )?;
        self.reconcile_lifecycle();
        self.touch();
        self.push_projected_event(session_events::local_webrtc_answer_set(
            backend_id,
            production_ready,
            epoch.value(),
        ));
        Ok(())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn begin_webrtc_negotiation(
        &mut self,
        epoch: TransportEpoch,
    ) -> bool {
        if self.product_operations_closed() {
            return false;
        }
        if !self.transport.begin_primary(epoch) {
            return false;
        }
        self.signaling.begin_transport_generation();
        self.lifecycle.start_media();
        self.reconcile_lifecycle();
        self.touch();
        true
    }

    /// Append a local ICE candidate produced by the device-side WebRTC endpoint.
    pub(in crate::daemon::plugins::remote_desktop) fn record_local_ice_candidate(
        &mut self,
        candidate: Value,
    ) -> anyhow::Result<()> {
        if self.product_operations_closed() {
            return Ok(());
        }
        let transport_epoch = self.transport.active_epoch().ok_or_else(|| {
            anyhow::anyhow!("local ICE candidate requires an active transport epoch")
        })?;
        let candidate_count = self.signaling.push_local_ice_candidate(candidate.clone())?;
        self.touch();
        self.push_projected_event(session_events::local_ice_candidate(
            candidate,
            candidate_count,
            transport_epoch.value(),
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
        if self.product_operations_closed() {
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
        let payload = session_events::with_target_binding_context(self.target.binding(), payload);
        self.push_event(event_type, payload);
    }

    /// Record diagnostic input-channel activity without changing transport readiness.
    pub(in crate::daemon::plugins::remote_desktop) fn record_input_channel_event(
        &mut self,
        epoch: TransportEpoch,
        event_type: &str,
        payload: Value,
    ) {
        if self.product_operations_closed() {
            return;
        }
        self.touch();
        let payload = session_events::input_channel_diagnostic(
            self.target.binding(),
            epoch.value(),
            self.transport.media_transport_ready(),
            payload,
        );
        self.push_event(event_type, payload);
    }

    /// Store latest media stats and emit a bounded event-log row.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn record_media_stats(
        &mut self,
        epoch: TransportEpoch,
        stats: Value,
    ) {
        if self.lifecycle.is_terminal() || self.terminal_commit_in_progress {
            return;
        }
        if !self.transport.record_media_stats(epoch, stats.clone()) {
            return;
        }
        self.touch();
        self.push_projected_event(session_events::media_pipeline_stats(
            self.target.binding(),
            epoch.value(),
            self.transport.media_transport_ready(),
            stats,
        ));
    }

    pub(in crate::daemon::plugins::remote_desktop) fn merge_client_media_stats(
        &mut self,
        epoch: TransportEpoch,
        stats: Value,
    ) -> bool {
        if self.product_operations_closed()
            || !self.transport.merge_media_stats(epoch, stats.clone())
        {
            return false;
        }
        let merged_stats = self.transport.media_stats().unwrap_or(stats);
        self.touch();
        self.push_projected_event(session_events::media_pipeline_stats(
            self.target.binding(),
            epoch.value(),
            self.transport.media_transport_ready(),
            merged_stats,
        ));
        true
    }

    /// Detach preview transport state and return the stop signal to notify.
    pub(in crate::daemon::plugins::remote_desktop) fn detach_preview_transport(
        &mut self,
    ) -> Option<watch::Sender<bool>> {
        if self.terminal_commit_in_progress {
            return None;
        }
        self.transport.detach_preview_transport()
    }

    /// Detach an InvokeBidi preview worker that reached a normal terminal path.
    pub(in crate::daemon::plugins::remote_desktop) fn detach_preview_transport_from_worker(
        &mut self,
        epoch: PreviewTransportEpoch,
        reason: &str,
    ) -> Option<watch::Sender<bool>> {
        if self.product_operations_closed() || !self.transport.preview_attached() {
            return None;
        }
        let stop_tx = self.transport.detach_preview_transport_if_epoch(epoch)?;
        self.reconcile_lifecycle();
        self.touch();
        self.push_projected_event(session_events::preview_transport_detached(reason));
        Some(stop_tx)
    }

    /// Record an InvokeBidi preview failure without failing production media.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_preview_transport_failed(
        &mut self,
        epoch: PreviewTransportEpoch,
        reason: &str,
        message: String,
    ) -> Option<watch::Sender<bool>> {
        if self.product_operations_closed() || !self.transport.preview_attached() {
            return None;
        }
        let stop_tx = self.transport.detach_preview_transport_if_epoch(epoch)?;
        self.reconcile_lifecycle();
        self.touch();
        self.push_projected_event(session_events::preview_transport_failed(reason, message));
        Some(stop_tx)
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn install_preview_transport_for_test(
        &mut self,
        stop_tx: watch::Sender<bool>,
    ) -> PreviewTransportEpoch {
        self.transport.install_preview_transport_for_test(stop_tx)
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn set_lease_expires_at_for_test(
        &mut self,
        lease_expires_at_ms: u64,
    ) {
        self.lease.set_expires_at_for_test(lease_expires_at_ms);
    }

    /// Enter Closing before external transport teardown. While Closing, new
    /// product operations are rejected but the active media generation may
    /// still publish its final pipeline statistics.
    pub(in crate::daemon::plugins::remote_desktop) fn begin_close(&mut self, reason: &str) -> bool {
        if !self.lifecycle.begin_termination(reason) {
            return false;
        }
        self.reserve_target_operation();
        self.touch();
        self.push_projected_event(session_events::session_closing(reason));
        self.consent.expire();
        true
    }

    /// Commit the terminal state only after every owned transport worker has
    /// stopped. This is the sole point that emits the terminal receipt.
    pub(in crate::daemon::plugins::remote_desktop) fn finish_close(&mut self, reason: &str) {
        if !self.lifecycle.is_terminating() {
            return;
        }
        self.lifecycle.terminate_closed(reason);
        self.touch();
        let terminal_event = self.push_projected_event(session_events::session_closed(reason));
        self.terminal_receipt = Some(self.project_terminal_receipt(reason, &terminal_event));
        self.event_log.close();
    }

    /// Publish an explicit terminal failure after settlement ownership has
    /// moved into the process quarantine. This does not claim that the worker
    /// stopped; it closes product operations while the resource owner remains
    /// retained for controlled runtime recovery.
    pub(in crate::daemon::plugins::remote_desktop) fn finish_failed_termination(
        &mut self,
        reason: &str,
    ) {
        if !self.lifecycle.is_terminating() {
            return;
        }
        let termination_intent = self.lifecycle.end_reason().map(ToString::to_string);
        self.lifecycle.terminate_failed(reason);
        self.touch();
        let terminal_event = self.push_projected_event(session_events::session_failed(
            reason,
            termination_intent.as_deref(),
        ));
        self.terminal_receipt = Some(self.project_terminal_receipt(reason, &terminal_event));
        self.event_log.close();
    }

    pub(in crate::daemon::plugins::remote_desktop) fn is_terminating(&self) -> bool {
        self.lifecycle.is_terminating()
    }

    /// Freeze the exact Closing revision selected for durable promotion.
    /// Returns false when another finalizer already owns the revision.
    pub(in crate::daemon::plugins::remote_desktop) fn begin_terminal_commit(&mut self) -> bool {
        if !self.lifecycle.is_terminating() || self.terminal_commit_in_progress {
            return false;
        }
        self.terminal_commit_in_progress = true;
        true
    }

    pub(in crate::daemon::plugins::remote_desktop) fn abort_terminal_commit(&mut self) {
        self.terminal_commit_in_progress = false;
    }

    pub(in crate::daemon::plugins::remote_desktop) fn terminal_commit_in_progress(&self) -> bool {
        self.terminal_commit_in_progress
    }

    fn begin_close_after_permission_revoked(&mut self) {
        if !self
            .lifecycle
            .begin_termination(REASON_TARGET_PERMISSION_REVOKED)
        {
            return;
        }
        self.touch();
        self.push_projected_event(session_events::session_closing(
            REASON_TARGET_PERMISSION_REVOKED,
        ));
    }

    /// Mark the session closed because its lease elapsed.
    pub(in crate::daemon::plugins::remote_desktop) fn expire(&mut self, now: u64) {
        if !self.begin_expiration(now) {
            return;
        }
        self.finish_expiration(now);
    }

    pub(in crate::daemon::plugins::remote_desktop) fn begin_expiration(
        &mut self,
        now: u64,
    ) -> bool {
        if !self.lifecycle.begin_termination(REASON_SESSION_EXPIRED) {
            return false;
        }
        self.reserve_target_operation();
        self.consent.expire();
        self.lease.touch(now);
        self.push_projected_event(session_events::session_closing(REASON_SESSION_EXPIRED));
        true
    }

    pub(in crate::daemon::plugins::remote_desktop) fn finish_expiration(&mut self, now: u64) {
        if !self.lifecycle.is_terminating() {
            return;
        }
        self.lifecycle.terminate_closed(REASON_SESSION_EXPIRED);
        self.lease.touch(now);
        let terminal_event = self.push_projected_event(session_events::session_expired(
            REASON_SESSION_EXPIRED,
            self.lease.expires_at_ms(),
        ));
        self.terminal_receipt =
            Some(self.project_terminal_receipt(REASON_SESSION_EXPIRED, &terminal_event));
        self.event_log.close();
    }

    /// Complete a durable Closing intent after daemon restart. Process-local
    /// transports cannot survive rehydration, so no media worker remains to
    /// settle; the original reason determines the canonical terminal event.
    pub(in crate::daemon::plugins::remote_desktop) fn finish_recovered_termination(
        &mut self,
        now: u64,
    ) {
        let Some(reason) = self.lifecycle.end_reason().map(ToString::to_string) else {
            return;
        };
        if reason == REASON_SESSION_EXPIRED {
            self.finish_expiration(now);
        } else {
            self.finish_close(&reason);
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn termination_reason(&self) -> Option<&str> {
        self.lifecycle.end_reason()
    }

    /// Mark the active production endpoint as accepting encoded media.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_webrtc_media_sending(
        &mut self,
        epoch: TransportEpoch,
        endpoint_ura: String,
    ) {
        if self.product_operations_closed() || !self.transport.mark_device_sending(epoch) {
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
        client_media_stats: Option<Value>,
    ) -> bool {
        if self.product_operations_closed() {
            return false;
        }
        if self.transport_epoch() != Some(epoch.value()) {
            return false;
        }
        let changed = match state {
            "presenting" => self.transport.mark_client_presenting(epoch),
            "stalled" | "detached" => self.transport.mark_client_stalled(epoch),
            _ => false,
        };
        let stats_recorded = client_media_stats
            .map(|stats| self.merge_client_media_stats(epoch, stats))
            .unwrap_or(false);
        if !changed && !stats_recorded && state != "presenting" {
            return false;
        }
        if changed {
            self.reconcile_lifecycle();
            self.touch();
            self.push_projected_event(session_events::client_media_state_changed(
                state,
                epoch.value(),
            ));
            if let Some(phase @ PrimaryMediaPhase::Degraded) = self.transport.primary_phase() {
                self.push_projected_event(session_events::session_degraded(
                    state,
                    epoch.value(),
                    phase.as_str(),
                ));
            }
        }
        true
    }

    /// Retire one failed WebRTC generation while preserving the product
    /// session for an authenticated offer on a newer transport epoch.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_webrtc_generation_failed_with_context(
        &mut self,
        epoch: TransportEpoch,
        event_kind: session_events::WebRtcFailureEventKind,
        reason: &str,
        message: String,
        context: serde_json::Value,
    ) -> bool {
        if self.product_operations_closed() || !self.transport.accepts_epoch(epoch) {
            return false;
        }
        if !self.transport.mark_failed(epoch) {
            return false;
        }
        self.lifecycle.suspend();
        self.signaling.set_webrtc_error(reason);
        self.touch();
        self.push_projected_event(session_events::webrtc_failed_with_context(
            event_kind,
            reason,
            message,
            epoch.value(),
            context,
        ));
        true
    }
}

fn recovery_terminal_reason(terminal_receipt: Option<&Value>) -> String {
    terminal_receipt
        .and_then(|receipt| receipt.get("reason_code").or_else(|| receipt.get("reason")))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "daemon_restart_rehydrated_terminal".to_string())
}

pub(in crate::daemon::plugins::remote_desktop) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::daemon::plugins::remote_desktop::constants::{
        direct_webrtc_endpoint_ura, REASON_SESSION_EXPIRED, REASON_TARGET_PERMISSION_REVOKED,
    };
    use crate::daemon::plugins::remote_desktop::relay_lease::{
        RemoteDesktopRelayLease, RemoteDesktopRelayLeaseAvailability, RemoteDesktopRelayLeaseInit,
        EASYNET_RELAY_PROVIDER,
    };
    use crate::daemon::plugins::remote_desktop::session::{
        RemoteDesktopRelayLeaseRotation, RemoteDesktopSession, RemoteDesktopState,
        TargetMediaSourceLost,
    };
    use crate::daemon::plugins::remote_desktop::session_consent_state::RemoteDesktopConsentPhase;
    use crate::daemon::plugins::remote_desktop::session_recovery::RemoteDesktopRecoverySnapshot;
    use crate::daemon::plugins::remote_desktop::session_state::RemoteDesktopSessionPhase;
    use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
    use crate::daemon::plugins::remote_desktop::target::{
        AppWindowSetProof, TargetGeometry, TargetResolutionError,
    };
    use crate::daemon::plugins::remote_desktop::target_tracking::{
        TargetObservation, TargetVisibilityState,
    };
    use crate::daemon::plugins::remote_desktop::test_support::{
        admit_decoded_video_for_test, test_application_session_init, test_session_init,
        test_window_session_init,
    };
    use crate::daemon::plugins::remote_desktop::view::serialize_session;

    fn relay_lease(
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
                username: format!("user-{lease_id}"),
                credential: format!("secret-{lease_id}"),
                issued_at_ms: 1,
                refresh_after_ms: 2,
                expires_at_ms: 3,
            },
        )
        .expect("test relay lease")
    }

    fn assert_target_tracking_payload_context(event: &Value, session: &RemoteDesktopSession) {
        let payload = &event["payload"];
        assert_eq!(
            payload["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            payload["binding_id"],
            json!(session.target_binding().binding_id())
        );
        assert_eq!(
            payload["binding_epoch"],
            json!(session.target_binding().binding_epoch())
        );
        assert_eq!(
            payload["target_identity_epoch"],
            event["target_identity_epoch"]
        );
        assert_eq!(
            payload["target_geometry_revision"],
            event["target_geometry_revision"]
        );
        assert_eq!(
            payload["media_source_epoch"],
            json!(session.target_binding().media_source_epoch())
        );
        assert_eq!(
            payload["consent_epoch"],
            json!(session.target_binding().consent_epoch())
        );
        assert_eq!(
            payload["target_binding"]["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            payload["target_binding"]["binding_id"],
            json!(session.target_binding().binding_id())
        );
        assert_eq!(
            payload["target_binding"]["media_source_epoch"],
            json!(session.target_binding().media_source_epoch())
        );
        assert_eq!(
            payload["target_binding"]["target_identity_epoch"],
            payload["target_identity_epoch"]
        );
        assert_eq!(
            payload["target_binding"]["target_geometry_revision"],
            payload["target_geometry_revision"]
        );
        assert_eq!(payload["target_binding"]["bounds"], payload["geometry"]);
        assert_eq!(
            payload["scope_audit"]["input_scope_reason"],
            payload["target_binding"]["input_scope_reason"]
        );
        assert!(
            payload["latest_target_diagnostic"].is_object(),
            "target lifecycle payload must include latest target diagnostic context"
        );
    }

    #[test]
    fn relay_rotation_returns_exact_resource_owner_for_success_and_rejection() {
        let session_id = "rd-relay-ownership";
        let resource_ura = "easynet:///r/acme/resource/display.test";
        let mut session = RemoteDesktopSession::new(test_session_init(
            session_id,
            resource_ura,
            vec!["webrtc".into()],
        ));
        session.install_relay_lease(RemoteDesktopRelayLeaseAvailability::Active(relay_lease(
            "lease-1",
            session_id,
            resource_ura,
        )));

        assert!(matches!(
            session.rotate_relay_lease_if_current(
                "lease-1",
                relay_lease("lease-2", session_id, resource_ura),
            ),
            RemoteDesktopRelayLeaseRotation::Installed
        ));
        assert_eq!(
            session
                .active_relay_lease()
                .expect("refreshed lease remains session-owned")
                .lease_id(),
            "lease-2"
        );

        assert!(matches!(
            session.rotate_relay_lease_if_current(
                "lease-1",
                relay_lease("lease-2", session_id, resource_ura),
            ),
            RemoteDesktopRelayLeaseRotation::AlreadyOwned
        ));

        let RemoteDesktopRelayLeaseRotation::Unowned(unattached) = session
            .rotate_relay_lease_if_current(
                "lease-1",
                relay_lease("lease-3", session_id, resource_ura),
            )
        else {
            panic!("stale distinct refresh must return its unowned lease");
        };
        assert_eq!(unattached.lease_id(), "lease-3");
        assert_eq!(
            session
                .active_relay_lease()
                .expect("stale refresh preserves current lease")
                .lease_id(),
            "lease-2"
        );

        let expired = session
            .retire_relay_lease_if_current("lease-2", "test_expired")
            .expect("current lease retires");
        assert_eq!(expired.lease_id(), "lease-2");
        assert!(session.active_relay_lease().is_none());
    }

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
            .any(|event| event["event_type"] == json!("TARGET_MOVED")));
        assert!(events
            .iter()
            .any(|event| event["event_type"] == json!("TARGET_RESIZED")));
        let geometry_events: Vec<_> = events
            .iter()
            .filter(|event| {
                event["event_type"] == json!("TARGET_MOVED")
                    || event["event_type"] == json!("TARGET_RESIZED")
            })
            .collect();
        assert_eq!(geometry_events.len(), 2);
        assert_eq!(geometry_events[0]["event_type"], json!("TARGET_MOVED"));
        assert_eq!(geometry_events[1]["event_type"], json!("TARGET_RESIZED"));
        assert_eq!(
            geometry_events[0]["target_geometry_revision"],
            geometry_events[1]["target_geometry_revision"]
        );
        assert_eq!(
            geometry_events[1]["sequence"].as_u64().unwrap(),
            geometry_events[0]["sequence"].as_u64().unwrap() + 1,
            "combined geometry observation must expand into monotonic ordered event-log rows"
        );
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
    fn rehydrated_non_terminal_session_can_start_new_media_epoch_without_new_session() {
        let mut source = RemoteDesktopSession::new(test_session_init(
            "rd-rehydrate-media-resume",
            "easynet:///r/acme/resource/display.rehydrate-media",
            vec!["webrtc".into()],
        ));
        assert!(source.begin_webrtc_negotiation(TransportEpoch::new(6)));
        let snapshot =
            RemoteDesktopRecoverySnapshot::from_session(&source).expect("snapshot derives");
        let mut session = RemoteDesktopSession::rehydrate(&snapshot).expect("session rehydrates");
        let epoch = TransportEpoch::new(7);
        let endpoint_ura = direct_webrtc_endpoint_ura("rd-rehydrate-media-resume");

        assert_eq!(session.session_id(), "rd-rehydrate-media-resume");
        assert_eq!(session.state(), RemoteDesktopState::Degraded);
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::Suspended
        );
        assert!(!session.media_transport_ready());
        assert_eq!(session.transport_epoch_high_watermark(), 6);
        assert!(!session.begin_webrtc_negotiation(TransportEpoch::new(6)));

        assert!(session.begin_webrtc_negotiation(epoch));
        session
            .set_local_webrtc_answer(
                epoch,
                json!({"type": "answer", "sdp": "v=0\r\n", "media_scope": "video_only"}),
                "plugin.macos.screencapturekit.videotoolbox.webrtc.v1",
                true,
                endpoint_ura.clone(),
            )
            .expect("local answer records on rehydrated session");
        session.mark_webrtc_media_sending(epoch, endpoint_ura);
        assert!(session.report_client_media_state(epoch, "presenting", None));
        admit_decoded_video_for_test(&mut session, epoch, "test-production-pipeline");

        assert_eq!(session.session_id(), "rd-rehydrate-media-resume");
        assert_eq!(session.transport_epoch(), Some(epoch.value()));
        assert_eq!(session.state(), RemoteDesktopState::Connected);
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::MediaActive
        );
        assert!(session.media_transport_ready());
        assert!(session.client_media_ready());
        assert!(session.production_media_ready());
        let rehydrated_event = session
            .events()
            .into_iter()
            .find(|event| event["event_type"] == json!("SESSION_REHYDRATED"))
            .expect("rehydration event is recorded");
        assert_eq!(rehydrated_event["recoverability"], json!("retry_session"));
        assert_eq!(
            rehydrated_event["subject_ura"],
            json!("easynet:///r/acme/resource/display.rehydrate-media")
        );
        assert_eq!(
            rehydrated_event["payload"]["target_binding"]["subject_ura"],
            json!("easynet:///r/acme/resource/display.rehydrate-media")
        );
        assert!(rehydrated_event["binding_epoch"].as_u64().unwrap_or(0) > 0);
        assert!(
            rehydrated_event["target_identity_epoch"]
                .as_u64()
                .unwrap_or(0)
                > 0
        );
        assert!(
            rehydrated_event["target_geometry_revision"]
                .as_u64()
                .unwrap_or(0)
                > 0
        );
        assert!(rehydrated_event["media_source_epoch"].as_u64().unwrap_or(0) > 0);
        assert!(rehydrated_event["consent_epoch"].as_u64().unwrap_or(0) > 0);
    }

    #[test]
    fn rehydrated_non_terminal_session_preserves_runtime_input_block_reason() {
        let source = RemoteDesktopSession::new(test_session_init(
            "rd-rehydrate-input-block",
            "easynet:///r/acme/resource/display.rehydrate-input-block",
            vec!["webrtc".into()],
        ));
        let snapshot =
            RemoteDesktopRecoverySnapshot::from_session(&source).expect("snapshot derives");
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

        let session = RemoteDesktopSession::rehydrate(&snapshot).expect("session rehydrates");

        assert_eq!(
            session.input_runtime_block_reason(),
            Some("accessibility_permission_denied")
        );
        assert_eq!(session.state(), RemoteDesktopState::Degraded);
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::Suspended
        );
    }

    #[test]
    fn closing_recovery_snapshot_preserves_reason_and_cannot_resume_as_degraded() {
        let mut source = RemoteDesktopSession::new(test_session_init(
            "rd-rehydrate-closing",
            "easynet:///r/acme/resource/display.rehydrate-closing",
            vec!["webrtc".into()],
        ));
        assert!(source.begin_close("caller_ended"));
        let snapshot =
            RemoteDesktopRecoverySnapshot::from_session(&source).expect("Closing snapshot derives");

        assert_eq!(snapshot.lifecycle_state(), "closing");
        assert_eq!(snapshot.termination_reason(), Some("caller_ended"));
        let mut session =
            RemoteDesktopSession::rehydrate(&snapshot).expect("Closing snapshot rehydrates");

        assert!(session.is_terminating());
        assert_eq!(session.state(), RemoteDesktopState::Closing);
        assert_eq!(session.termination_reason(), Some("caller_ended"));
        assert!(!session.begin_webrtc_negotiation(TransportEpoch::new(1)));

        session.finish_recovered_termination(super::now_ms());
        assert!(session.is_terminal());
        assert_eq!(session.state(), RemoteDesktopState::Closed);
        assert_eq!(session.end_reason(), Some("caller_ended"));
        assert_eq!(
            session.terminal_receipt().expect("terminal receipt")["reason_code"],
            json!("caller_ended")
        );
    }

    #[test]
    fn session_close_events_project_terminal_reason_code() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-close-event",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(73);
        session.begin_webrtc_negotiation(epoch);

        assert!(session.begin_close("caller_ended"));
        assert!(session.is_terminating());
        assert_eq!(session.state(), RemoteDesktopState::Closing);
        assert!(session.terminal_receipt().is_none());
        assert!(!session
            .events()
            .iter()
            .any(|event| event["event_type"] == json!("SESSION_CLOSED")));
        session.record_media_stats(
            epoch,
            json!({"terminal": true, "audio_packets_written": 12}),
        );
        assert_eq!(
            session
                .media_stats()
                .expect("Closing accepts final media stats")["terminal"],
            json!(true)
        );
        session.finish_close("caller_ended");
        session.record_media_stats(epoch, json!({"terminal": false}));
        assert_eq!(
            session.media_stats().expect("terminal stats remain frozen")["terminal"],
            json!(true)
        );

        let events = session.events();
        let closing_index = events
            .iter()
            .position(|event| event["event_type"] == json!("SESSION_CLOSING"))
            .expect("SESSION_CLOSING event");
        let closed_index = events
            .iter()
            .position(|event| event["event_type"] == json!("SESSION_CLOSED"))
            .expect("SESSION_CLOSED event");
        assert!(
            closing_index < closed_index,
            "closing event must precede terminal close"
        );
        let closing = &events[closing_index];
        assert_eq!(closing["reason_code"], json!("caller_ended"));
        assert_eq!(closing["recoverability"], json!("closing"));
        assert_eq!(
            closing["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            closing["binding_id"],
            json!(session.target_binding().binding_id())
        );
        assert_eq!(
            closing["target_identity_epoch"],
            json!(session.target_binding().target_identity_epoch())
        );
        assert_eq!(closing["payload"]["reason_code"], json!("caller_ended"));
        assert_eq!(closing["payload"]["recoverability"], json!("closing"));
        assert_eq!(
            closing["payload"]["target_binding"]["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(closing["terminal"], json!(false));

        let closed = &events[closed_index];
        assert_eq!(closed["reason_code"], json!("caller_ended"));
        assert_eq!(closed["recoverability"], json!("closed"));
        assert_eq!(
            closed["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            closed["target_geometry_revision"],
            json!(session.target_binding().target_geometry_revision())
        );
        assert_eq!(closed["payload"]["reason_code"], json!("caller_ended"));
        assert_eq!(closed["payload"]["recoverability"], json!("closed"));
        assert_eq!(
            closed["payload"]["target_binding"]["binding_id"],
            json!(session.target_binding().binding_id())
        );
        assert_eq!(closed["terminal"], json!(true));

        let terminal_receipt = session
            .terminal_receipt()
            .expect("caller close must project a terminal receipt");
        assert_eq!(
            terminal_receipt["receipt_type"],
            json!("remoteapp.session.terminal.v1")
        );
        assert_eq!(terminal_receipt["session_id"], json!("rd-close-event"));
        assert_eq!(terminal_receipt["reason_code"], json!("caller_ended"));
        assert_eq!(
            terminal_receipt["terminal_event_id"],
            closed["event_id"].clone()
        );
        assert_eq!(
            terminal_receipt["terminal_event_sequence"],
            closed["sequence"].clone()
        );
        assert_eq!(terminal_receipt["terminal"], json!(true));
    }

    #[test]
    fn initial_session_events_project_reason_codes_in_order() {
        let session = RemoteDesktopSession::new(test_session_init(
            "rd-initial-events",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));

        let events = session.events();
        let created_index = events
            .iter()
            .position(|event| event["event_type"] == json!("SESSION_CREATED"))
            .expect("SESSION_CREATED event");
        let resolved_index = events
            .iter()
            .position(|event| event["event_type"] == json!("CAPTURE_TARGET_RESOLVED"))
            .expect("CAPTURE_TARGET_RESOLVED event");
        let bound_index = events
            .iter()
            .position(|event| event["event_type"] == json!("TARGET_BOUND"))
            .expect("TARGET_BOUND event");
        assert!(
            created_index < resolved_index && resolved_index < bound_index,
            "initial events must keep session, resolution, and binding order"
        );

        let created = &events[created_index];
        assert_eq!(created["reason_code"], json!("session_created"));
        assert_eq!(created["recoverability"], json!("continue"));
        assert_eq!(
            created["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            created["payload"]["target_binding"]["binding_id"],
            json!(session.target_binding().binding_id())
        );
        assert_eq!(created["payload"]["reason_code"], json!("session_created"));
        assert_eq!(created["payload"]["recoverability"], json!("continue"));

        let resolved = &events[resolved_index];
        assert_eq!(resolved["reason_code"], json!("capture_target_resolved"));
        assert_eq!(resolved["recoverability"], json!("continue"));
        assert_eq!(
            resolved["payload"]["reason_code"],
            json!("capture_target_resolved")
        );
        assert_eq!(resolved["payload"]["recoverability"], json!("continue"));
        assert_eq!(
            resolved["subject_ura"],
            json!("easynet:///r/acme/resource/display.test")
        );
        assert_eq!(
            resolved["binding_id"],
            json!(session.target_binding().binding_id())
        );
        assert_eq!(
            resolved["binding_epoch"],
            json!(session.target_binding().binding_epoch())
        );
        assert_eq!(resolved["previous_target_identity_epoch"], json!(null));
        assert_eq!(
            resolved["target_identity_epoch"],
            json!(session.target_binding().target_identity_epoch())
        );
        assert_eq!(
            resolved["target_geometry_revision"],
            json!(session.target_binding().target_geometry_revision())
        );
        assert_eq!(
            resolved["media_source_epoch"],
            json!(session.target_binding().media_source_epoch())
        );
        assert_eq!(
            resolved["consent_epoch"],
            json!(session.target_binding().consent_epoch())
        );

        let bound = &events[bound_index];
        assert_eq!(bound["reason_code"], json!("target_bound"));
        assert_eq!(bound["recoverability"], json!("continue"));
        assert_eq!(
            bound["consent_epoch"],
            json!(session.target_binding().consent_epoch())
        );
    }

    #[test]
    fn session_expiry_events_project_terminal_reason_code() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-expire-event",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));

        session.expire(42);

        let event = session
            .events()
            .into_iter()
            .find(|event| event["event_type"] == json!("SESSION_CLOSED"))
            .expect("SESSION_CLOSED event");
        assert_eq!(event["reason_code"], json!(REASON_SESSION_EXPIRED));
        assert_eq!(event["recoverability"], json!("closed"));
        assert_eq!(
            event["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            event["media_source_epoch"],
            json!(session.target_binding().media_source_epoch())
        );
        assert_eq!(
            event["payload"]["reason_code"],
            json!(REASON_SESSION_EXPIRED)
        );
        assert_eq!(event["payload"]["recoverability"], json!("closed"));
        assert_eq!(
            event["payload"]["target_binding"]["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(event["terminal"], json!(true));

        let terminal_receipt = session
            .terminal_receipt()
            .expect("lease expiry must project a terminal receipt");
        assert_eq!(
            terminal_receipt["receipt_type"],
            json!("remoteapp.session.terminal.v1")
        );
        assert_eq!(
            terminal_receipt["reason_code"],
            json!(REASON_SESSION_EXPIRED)
        );
        assert_eq!(
            terminal_receipt["terminal_event_id"],
            event["event_id"].clone()
        );
        assert_eq!(
            terminal_receipt["terminal_event_sequence"],
            event["sequence"].clone()
        );
        assert_eq!(terminal_receipt["terminal"], json!(true));
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

        session.record_target_observation(TargetObservation::TitleChanged {
            title: Some("Renamed display".to_string()),
            observed_at_ms: 100,
        });

        let events = session.events();
        let target_event = events
            .iter()
            .find(|event| event["event_type"] == json!("TARGET_TITLE_CHANGED"))
            .expect("TARGET_TITLE_CHANGED event");
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
        assert_target_tracking_payload_context(target_event, &session);
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
            events[media_source_lost_index]["consent_epoch"],
            json!(session.target_binding().consent_epoch())
        );
        assert_eq!(
            events[media_source_lost_index]["payload"]["frontend_action"],
            json!("refresh_targets")
        );
        assert!(
            events
                .iter()
                .all(|event| event["event_type"] != json!("SESSION_DEGRADED")),
            "target-domain media source loss must not be collapsed into session degradation"
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

        assert!(session.report_client_media_state(epoch, "presenting", None));
        assert!(session.mark_input_frame_applied(epoch));
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
    fn client_media_stall_emits_session_degraded_recovery_event() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-client-media-stalled",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(29);

        session.begin_webrtc_negotiation(epoch);
        session.mark_webrtc_media_sending(
            epoch,
            direct_webrtc_endpoint_ura("rd-client-media-stalled"),
        );
        assert!(session.report_client_media_state(epoch, "presenting", None));
        assert_eq!(session.state(), RemoteDesktopState::Connected);
        assert_eq!(
            session.transport_state()["primary"],
            json!("client_presenting")
        );

        assert!(session.report_client_media_state(epoch, "stalled", None));
        assert_eq!(session.state(), RemoteDesktopState::Degraded);
        assert_eq!(session.transport_state()["primary"], json!("degraded"));
        assert!(session.media_transport_ready());
        assert!(!session.client_media_ready());
        assert!(!session.production_media_ready());

        let events = session.events();
        let client_state_index = events
            .iter()
            .position(|event| event["event_type"] == json!("CLIENT_MEDIA_STALLED"))
            .expect("CLIENT_MEDIA_STALLED event");
        let degraded_index = events
            .iter()
            .position(|event| event["event_type"] == json!("SESSION_DEGRADED"))
            .expect("SESSION_DEGRADED event");
        assert!(
            client_state_index < degraded_index,
            "client media state cause must precede lifecycle degradation projection"
        );

        let stalled = &events[client_state_index];
        assert_eq!(stalled["reason_code"], json!("client_media_stalled"));
        assert_eq!(stalled["recoverability"], json!("retry_session"));
        assert_eq!(stalled["transport_epoch"], json!(epoch.value()));

        let degraded = &events[degraded_index];
        assert_eq!(degraded["state"], json!("degraded"));
        assert_eq!(
            degraded["event_type_proto"],
            json!("REMOTE_DESKTOP_EVENT_STATE_CHANGED")
        );
        assert_eq!(degraded["reason_code"], json!("client_media_stalled"));
        assert_eq!(degraded["recoverability"], json!("retry_session"));
        assert_eq!(degraded["transport_epoch"], json!(epoch.value()));
        assert_eq!(degraded["payload"]["failure_domain"], json!("client_media"));
        assert_eq!(
            degraded["payload"]["frontend_action"],
            json!("retry_session")
        );
        assert_eq!(degraded["payload"]["primary_phase"], json!("degraded"));
        assert_eq!(degraded["payload"]["media_transport_ready"], json!(true));
        assert_eq!(degraded["payload"]["client_media_ready"], json!(false));
        assert_eq!(degraded["terminal"], json!(false));
    }

    #[test]
    fn client_media_report_merges_transport_evidence_without_overwriting_device_stats() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-client-transport-evidence",
            "easynet:///r/acme/resource/display.client-transport",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(41);

        session.begin_webrtc_negotiation(epoch);
        session.record_media_stats(
            epoch,
            json!({
                "backend_id": "macos-sck-videotoolbox-webrtc",
                "target_fps": 60,
                "webrtc_stats": {
                    "outbound_rtp": {
                        "frames_sent": 10
                    }
                }
            }),
        );
        session.mark_webrtc_media_sending(
            epoch,
            direct_webrtc_endpoint_ura("rd-client-transport-evidence"),
        );

        assert!(session.report_client_media_state(
            epoch,
            "presenting",
            Some(json!({
                "webrtc_stats": {
                    "selected_candidate_pair": {
                        "id": "pair-1",
                        "candidate_pair_id": "pair-1",
                        "local_candidate_id": "local-1",
                        "remote_candidate_id": "remote-1",
                        "selected_route_class": "direct",
                        "state": "succeeded",
                        "selected": true,
                        "nominated": true
                    }
                },
                "browser_stats": {
                    "frames_decoded": 12,
                    "frame_width": 1280,
                    "frame_height": 720
                },
                "render_probe": {
                    "probe_source": "browser_webrtc_receiver",
                    "selected_resource_ura": session.target_binding().subject_ura(),
                    "session_id": "rd-client-transport-evidence",
                    "media_pipeline_id": "macos-sck-videotoolbox-webrtc",
                    "video_codec": "h264",
                    "video_transport": "webrtc",
                    "observed_at_ms": 1787470677805u64,
                    "decoded_video_frames": 12,
                    "frame_width": 1280,
                    "frame_height": 720
                }
            }))
        ));
        assert!(session.report_client_media_state(
            epoch,
            "presenting",
            Some(json!({
                "browser_stats": {
                    "frames_decoded": 24,
                    "decode_avg_ms": 5.0
                }
            }))
        ));

        let stats = session.media_stats().expect("merged media stats");
        assert_eq!(stats["backend_id"], json!("macos-sck-videotoolbox-webrtc"));
        assert_eq!(stats["target_fps"], json!(60));
        assert_eq!(
            stats["webrtc_stats"]["outbound_rtp"]["frames_sent"],
            json!(10)
        );
        assert_eq!(
            stats["webrtc_stats"]["selected_candidate_pair"]["candidate_pair_id"],
            json!("pair-1")
        );
        assert_eq!(stats["browser_stats"]["frames_decoded"], json!(24));
        assert_eq!(stats["browser_stats"]["frame_width"], json!(1280));
        assert_eq!(stats["browser_stats"]["decode_avg_ms"], json!(5.0));
        assert_eq!(
            stats["render_probe"]["probe_source"],
            json!("browser_webrtc_receiver")
        );
        assert_eq!(
            stats["render_probe"]["selected_resource_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            stats["render_probe"]["media_pipeline_id"],
            json!("macos-sck-videotoolbox-webrtc")
        );

        let media_events: Vec<_> = session
            .events()
            .into_iter()
            .filter(|event| event["event_type"] == json!("MEDIA_PIPELINE_STATS"))
            .collect();
        assert_eq!(media_events.len(), 3);
        let latest = media_events.last().expect("latest media stats event");
        assert_eq!(
            latest["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            latest["binding_id"],
            json!(session.target_binding().binding_id())
        );
        assert_eq!(
            latest["target_geometry_revision"],
            json!(session.target_binding().target_geometry_revision())
        );
        assert_eq!(latest["transport_epoch"], json!(epoch.value()));
        assert_eq!(
            latest["payload"]["target_binding"]["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            latest["payload"]["stats"]["browser_stats"]["decode_avg_ms"],
            json!(5.0)
        );
        assert_eq!(
            latest["payload"]["stats"]["render_probe"]["decoded_video_frames"],
            json!(12)
        );
    }

    #[test]
    fn record_webrtc_diagnostic_projects_target_binding_context() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-webrtc-diagnostic-target",
            "easynet:///r/acme/resource/window.webrtc-diagnostic",
            vec!["webrtc".into()],
        ));

        session.record_webrtc_diagnostic(
            "WEBRTC_DIAGNOSTIC",
            Some("ice_disconnected".to_string()),
            json!({
                "ice_connection_state": "disconnected",
                "peer_connection_state": "connecting",
                "selected_candidate_pair": {
                    "candidate_pair_id": "pair-webrtc-1"
                }
            }),
        );

        let event = session
            .events()
            .into_iter()
            .find(|event| event["event_type"] == json!("WEBRTC_DIAGNOSTIC"))
            .expect("WEBRTC_DIAGNOSTIC event");
        assert_eq!(
            event["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            event["binding_epoch"],
            json!(session.target_binding().binding_epoch())
        );
        assert_eq!(
            event["target_identity_epoch"],
            json!(session.target_binding().target_identity_epoch())
        );
        assert_eq!(
            event["payload"]["target_binding"]["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            event["payload"]["diagnostic"]["selected_candidate_pair"]["candidate_pair_id"],
            json!("pair-webrtc-1")
        );
        assert_eq!(event["payload"]["webrtc_ice_state"], json!("disconnected"));
        assert_eq!(event["payload"]["webrtc_error"], json!("ice_disconnected"));
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
        assert!(session.report_client_media_state(epoch, "presenting", None));
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
        assert!(session.report_client_media_state(epoch, "presenting", None));
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
    fn runtime_input_permission_block_deactivates_input_without_failing_media() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-input-permission-blocked",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(21);

        session.begin_webrtc_negotiation(epoch);
        session.mark_webrtc_media_sending(
            epoch,
            direct_webrtc_endpoint_ura("rd-input-permission-blocked"),
        );
        assert!(session.report_client_media_state(epoch, "presenting", None));
        assert!(session.activate_input_for_transport_epoch(epoch));
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::InputActive
        );

        assert!(
            session.block_input_for_runtime_permission(epoch, "accessibility_permission_denied")
        );
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::MediaActive
        );
        assert_eq!(session.state(), RemoteDesktopState::Connected);
        assert!(session.media_transport_ready());
        assert!(session.client_media_ready());
        assert_eq!(
            session.input_runtime_block_reason(),
            Some("accessibility_permission_denied")
        );
        assert!(
            !session.block_input_for_runtime_permission(epoch, "accessibility_permission_denied"),
            "permission block projection must be edge-triggered"
        );
        assert!(session.mark_input_frame_applied(epoch));
        assert_eq!(
            session.input_runtime_block_reason(),
            None,
            "runtime input block reason must clear only after input is proven active again"
        );

        let blocked_events: Vec<_> = session
            .events()
            .into_iter()
            .filter(|event| event["event_type"] == json!("INPUT_PERMISSION_BLOCKED"))
            .collect();
        assert_eq!(blocked_events.len(), 1);
        let blocked = &blocked_events[0];
        assert_eq!(blocked["state"], json!("connected"));
        assert_eq!(
            blocked["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            blocked["target_identity_epoch"],
            json!(session.target_binding().target_identity_epoch())
        );
        assert_eq!(blocked["transport_epoch"], json!(epoch.value()));
        assert_eq!(
            blocked["payload"]["reason"],
            json!("accessibility_permission_denied")
        );
        assert_eq!(
            blocked["payload"]["target_binding"]["binding_id"],
            json!(session.target_binding().binding_id())
        );
        assert_eq!(
            blocked["payload"]["frontend_action"],
            json!("request_permission")
        );
        assert_eq!(blocked["payload"]["media_transport_ready"], json!(true));

        let restored_events: Vec<_> = session
            .events()
            .into_iter()
            .filter(|event| event["event_type"] == json!("INPUT_PERMISSION_RESTORED"))
            .collect();
        assert_eq!(restored_events.len(), 1);
        let restored = &restored_events[0];
        assert_eq!(restored["state"], json!("connected"));
        assert_eq!(
            restored["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            restored["payload"]["target_geometry_revision"],
            json!(session.target_binding().target_geometry_revision())
        );
        assert_eq!(restored["transport_epoch"], json!(epoch.value()));
        assert_eq!(restored["payload"]["input_activation"], json!("enabled"));
        assert_eq!(restored["payload"]["recoverability"], json!("resolved"));
        assert_eq!(restored["payload"]["frontend_action"], Value::Null);
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
        assert!(session.report_client_media_state(epoch, "presenting", None));
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
        let pending = session
            .events()
            .into_iter()
            .find(|event| event["event_type"] == json!("TARGET_LOSS_PENDING"))
            .expect("pending target loss is an observable safety transition");
        assert_eq!(
            pending["payload"]["reason_code"],
            json!("target_loss_pending")
        );
        assert_eq!(pending["payload"]["input_enabled"], json!(false));

        let snapshot = RemoteDesktopRecoverySnapshot::from_session(&session)
            .expect("pending target-loss snapshot derives");
        let recovered = RemoteDesktopSession::rehydrate(&snapshot)
            .expect("pending target-loss safety state rehydrates");
        assert_eq!(
            recovered.target_tracking_state()["input_blocked_reason"],
            json!("target_loss_pending")
        );
    }

    #[test]
    fn monitor_unavailable_bypasses_target_loss_debounce_and_stops_media() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-monitor-unavailable",
            "easynet:///r/acme/resource/window.test",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(29);

        session.begin_webrtc_negotiation(epoch);
        session
            .mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura("rd-monitor-unavailable"));
        assert!(session.report_client_media_state(epoch, "presenting", None));
        assert!(session.activate_input_for_transport_epoch(epoch));

        let media_loss = session.record_target_observation(TargetObservation::MonitorUnavailable {
            detail: "target monitor restart budget exhausted".into(),
            observed_at_ms: 300,
        });

        assert_eq!(
            media_loss,
            Some(TargetMediaSourceLost {
                transport_epoch: epoch,
                reason: TargetResolutionError::CaptureBackendUnavailable,
            })
        );
        assert_eq!(session.target_tracking_state()["status"], json!("lost"));
        assert!(!session.media_transport_ready());
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::Suspended
        );
        assert!(session.events().into_iter().any(|event| {
            event["event_type"] == json!("TARGET_LOST")
                && event["payload"]["reason_code"] == json!("capture_backend_unavailable")
        }));

        let snapshot = RemoteDesktopRecoverySnapshot::from_session(&session)
            .expect("monitor-unavailable session snapshot derives");
        let recovered = RemoteDesktopSession::rehydrate(&snapshot)
            .expect("monitor-unavailable target state rehydrates");
        assert_eq!(recovered.target_tracking_state()["status"], json!("lost"));
        assert_eq!(
            recovered.target_tracking_state()["input_enabled"],
            json!(false)
        );
    }

    #[test]
    fn first_permission_denial_suspends_media_without_revoking_consent_or_session() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-permission-verification",
            "easynet:///r/acme/resource/display.permission-verification",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(16);
        session.begin_webrtc_negotiation(epoch);
        session.mark_webrtc_media_sending(
            epoch,
            direct_webrtc_endpoint_ura("rd-permission-verification"),
        );
        assert!(session.report_client_media_state(epoch, "presenting", None));
        assert!(session.activate_input_for_transport_epoch(epoch));

        let media_lost = session
            .record_target_observation(TargetObservation::PermissionVerificationRequired {
                detail: "screen-capture preflight returned false".to_string(),
                observed_at_ms: 100,
            })
            .expect("first denial retires active media");

        assert_eq!(media_lost.transport_epoch, epoch);
        assert_eq!(
            media_lost.reason,
            TargetResolutionError::TargetPermissionMissing
        );
        assert_eq!(session.consent_phase(), RemoteDesktopConsentPhase::Active);
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::Suspended
        );
        assert!(!session.is_terminating());
        assert!(!session.is_terminal());
        assert_eq!(
            session.target_tracking_state()["status"],
            json!("permission_verification_pending")
        );
        assert_eq!(
            session.target_tracking_state()["input_enabled"],
            json!(false)
        );
        assert!(session.events().iter().any(|event| {
            event["event_type"] == json!("TARGET_PERMISSION_VERIFICATION_PENDING")
        }));
        assert!(!session
            .events()
            .iter()
            .any(|event| { event["event_type"] == json!("TARGET_PERMISSION_REVOKED") }));

        let snapshot = RemoteDesktopRecoverySnapshot::from_session(&session)
            .expect("permission verification snapshot derives");
        let recovered = RemoteDesktopSession::rehydrate(&snapshot)
            .expect("permission verification snapshot rehydrates");
        assert_eq!(recovered.consent_phase(), RemoteDesktopConsentPhase::Active);
        assert_eq!(
            recovered.target_tracking_state()["status"],
            json!("permission_verification_pending")
        );
    }

    #[test]
    fn consent_revocation_terminates_session_and_blocks_input_activation() {
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
        assert!(session.report_client_media_state(epoch, "presenting", None));
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
            RemoteDesktopSessionPhase::Terminating
        );
        assert_eq!(session.state(), RemoteDesktopState::Closing);
        assert!(!session.is_terminal());
        assert!(!session.production_media_ready());
        assert!(
            !session.activate_input_for_transport_epoch(epoch),
            "Closing revoked consent must prevent input from reactivating even with the same transport epoch"
        );
        session.record_media_stats(epoch, json!({"terminal": true}));
        session.finish_close(REASON_TARGET_PERMISSION_REVOKED);
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::Terminated
        );
        assert_eq!(session.state(), RemoteDesktopState::Closed);
        assert_eq!(session.end_reason(), Some(REASON_TARGET_PERMISSION_REVOKED));
        assert!(session.is_terminal());

        let events = session.events();
        let permission_revoked_index = events
            .iter()
            .position(|event| event["event_type"] == json!("TARGET_PERMISSION_REVOKED"))
            .expect("TARGET_PERMISSION_REVOKED event");
        let media_source_lost_index = events
            .iter()
            .position(|event| event["event_type"] == json!("MEDIA_SOURCE_LOST"))
            .expect("MEDIA_SOURCE_LOST event");
        let session_closed_index = events
            .iter()
            .position(|event| event["event_type"] == json!("SESSION_CLOSED"))
            .expect("SESSION_CLOSED event");
        assert!(
            permission_revoked_index < media_source_lost_index,
            "consent/permission event must precede media-source stop projection"
        );
        assert!(
            media_source_lost_index < session_closed_index,
            "media-source stop projection must precede terminal session closure"
        );
        assert_eq!(
            events[permission_revoked_index]["payload"]["reason_code"],
            json!("target_permission_missing")
        );
        assert_eq!(
            events[media_source_lost_index]["payload"]["transport_epoch"],
            json!(epoch.value())
        );
        assert_eq!(
            events[session_closed_index]["payload"]["reason_code"],
            json!(REASON_TARGET_PERMISSION_REVOKED)
        );
        assert_eq!(
            session.terminal_receipt().unwrap()["reason_code"],
            json!(REASON_TARGET_PERMISSION_REVOKED)
        );
        assert_eq!(
            session.terminal_receipt().unwrap()["terminal_event_type"],
            json!("SESSION_CLOSED")
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
        session
            .set_local_webrtc_answer(
                epoch,
                json!({"type": "answer", "sdp": "v=0", "media_scope": "video_only"}),
                "sck-native",
                true,
                "easynet:///r/acme/ability/remote-desktop.transport".into(),
            )
            .expect("local answer records");
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
    fn production_readiness_reports_client_blocker_and_route_degradation_before_presentation() {
        let epoch = TransportEpoch::new(19);
        let endpoint_ura = direct_webrtc_endpoint_ura("rd-route-before-client");
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-route-before-client",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));

        session.begin_webrtc_negotiation(epoch);
        session
            .set_local_webrtc_answer(
                epoch,
                json!({"type": "answer", "sdp": "v=0", "media_scope": "video_only"}),
                "sck-native",
                true,
                endpoint_ura.clone(),
            )
            .expect("local answer records");
        session
            .record_local_ice_candidate(json!({
                "candidate": "candidate:1 1 UDP 2122252543 127.0.0.1 50000 typ host",
                "sdpMid": "0",
                "sdpMLineIndex": 0
            }))
            .expect("local host candidate records");
        session.mark_webrtc_media_sending(epoch, endpoint_ura);

        assert!(session.production_codec_negotiated());
        assert!(session.media_transport_ready());
        assert!(!session.client_media_ready());

        let view = serialize_session(&session);
        assert_eq!(
            view["production_readiness"]["production_route_ready"],
            json!(false)
        );
        assert_eq!(
            view["production_readiness"]["route_state"]["route_class"],
            json!("host_only")
        );
        assert_eq!(
            view["production_readiness"]["blocked_reason"],
            json!("client_media_not_presenting")
        );
        assert_eq!(
            view["production_readiness"]["route_readiness_blocker"]["unavailable_reason"],
            json!("host_only_no_nat_or_relay")
        );
        assert_eq!(
            view["production_readiness"]["route_readiness_blocker"]["recoverability"],
            json!("retry_session")
        );
        assert_eq!(
            view["production_readiness"]["route_readiness_blocker"]["frontend_action"],
            json!("retry_session")
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

        assert!(!session.report_client_media_state(epoch, "stalled", None));
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
        session
            .set_local_webrtc_answer(
                epoch,
                json!({"type": "answer", "sdp": "v=0", "media_scope": "video_only"}),
                "sck-native",
                true,
                "easynet:///r/acme/ability/remote-desktop.transport".into(),
            )
            .expect("local answer records");
        session.mark_webrtc_media_sending(
            epoch,
            direct_webrtc_endpoint_ura("rd-target-rebind-failed"),
        );
        assert!(session.report_client_media_state(epoch, "presenting", None));
        admit_decoded_video_for_test(&mut session, epoch, "test-production-pipeline");
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
            rebind_attempted["reason_code"],
            json!("target_rebind_attempted")
        );
        assert_eq!(rebind_attempted["recoverability"], json!("retry_session"));
        assert_eq!(rebind_attempted["state"], json!("suspended"));
        assert_eq!(
            rebind_attempted["event_type_proto"],
            json!("REMOTE_DESKTOP_EVENT_TARGET_CHANGED")
        );
        assert_eq!(
            rebind_attempted["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            rebind_attempted["binding_id"],
            json!(session.target_binding().binding_id())
        );
        assert_eq!(
            rebind_attempted["binding_epoch"],
            json!(session.target_binding().binding_epoch())
        );
        assert_eq!(
            rebind_attempted["target_identity_epoch"],
            json!(session.target_binding().target_identity_epoch())
        );
        assert_eq!(
            rebind_attempted["target_geometry_revision"],
            json!(session.target_binding().target_geometry_revision())
        );
        assert_eq!(
            rebind_attempted["media_source_epoch"],
            json!(session.target_binding().media_source_epoch())
        );
        assert_eq!(
            rebind_attempted["payload"]["target_status"],
            json!("rebinding")
        );
        assert_eq!(
            rebind_attempted["payload"]["frontend_action"],
            json!("retry_session")
        );
        assert_target_tracking_payload_context(rebind_attempted, &session);
        let rebind_failed = events
            .iter()
            .find(|event| event["event_type"] == json!("TARGET_REBIND_FAILED"))
            .expect("reappearance without explicit rebind policy must be typed");
        assert_eq!(rebind_failed["state"], json!("suspended"));
        assert_eq!(
            rebind_failed["reason_code"],
            json!("explicit_rebind_required")
        );
        assert_eq!(
            rebind_failed["recoverability"],
            json!("new_session_required")
        );
        assert_eq!(
            rebind_failed["subject_ura"],
            json!(session.target_binding().subject_ura())
        );
        assert_eq!(
            rebind_failed["binding_id"],
            json!(session.target_binding().binding_id())
        );
        assert_eq!(
            rebind_failed["binding_epoch"],
            json!(session.target_binding().binding_epoch())
        );
        assert_eq!(
            rebind_failed["target_identity_epoch"],
            json!(session.target_binding().target_identity_epoch())
        );
        assert_eq!(
            rebind_failed["target_geometry_revision"],
            json!(session.target_binding().target_geometry_revision())
        );
        assert_eq!(
            rebind_failed["media_source_epoch"],
            json!(session.target_binding().media_source_epoch())
        );
        assert_eq!(
            rebind_failed["event_type_proto"],
            json!("REMOTE_DESKTOP_EVENT_TARGET_CHANGED")
        );
        assert_eq!(
            rebind_failed["payload"]["reason_code"],
            json!("explicit_rebind_required")
        );
        assert_target_tracking_payload_context(rebind_failed, &session);
        assert_eq!(
            rebind_failed["payload"]["frontend_action"],
            json!("refresh_targets")
        );
        assert_eq!(rebind_failed["payload"]["target_status"], json!("lost"));
        assert_eq!(rebind_failed["payload"]["input_enabled"], json!(false));
    }

    #[test]
    fn target_rebind_deadline_expiry_rejects_session_rebinding() {
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-target-rebind-deadline",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));

        session.record_target_observation(TargetObservation::Lost {
            reason: TargetResolutionError::TargetNotFound,
            detail: "target disappeared".into(),
            observed_at_ms: 100,
        });
        session.record_target_observation(TargetObservation::Lost {
            reason: TargetResolutionError::TargetNotFound,
            detail: "target still disappeared".into(),
            observed_at_ms: 1_200,
        });
        assert_eq!(session.target_tracking_state()["status"], json!("lost"));

        assert!(
            session
                .record_target_observation(TargetObservation::VisibilityChanged {
                    visibility_state: TargetVisibilityState::Visible,
                    target_geometry_revision: 9,
                    observed_at_ms: 1_300,
                })
                .is_none(),
            "rebind attempt must not stop a media source when none is active"
        );
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::Rebinding
        );

        assert!(
            session.expire_target_rebind_deadline(31_299).is_none(),
            "session must not reject rebinding before the published deadline"
        );
        let expiration = session
            .expire_target_rebind_deadline(31_300)
            .expect("session expires the bounded rebind attempt");
        assert!(expiration.into_media_source_lost().is_none());

        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::Suspended
        );
        assert_eq!(
            session.target_tracking_state()["status"],
            json!("lost"),
            "deadline expiry must terminate the target projection"
        );
        assert_eq!(
            session.target_tracking_state()["input_enabled"],
            json!(false)
        );
        let events = session.events();
        let rebind_failed = events
            .iter()
            .find(|event| event["event_type"] == json!("TARGET_REBIND_FAILED"))
            .expect("deadline expiry emits target rebind failure");
        assert_eq!(
            rebind_failed["reason_code"],
            json!("explicit_rebind_required")
        );
        assert_eq!(
            rebind_failed["payload"]["detail"],
            json!("rebind_window_expired")
        );
        assert_eq!(
            rebind_failed["payload"]["rebind_deadline_ms"],
            json!(31_300)
        );
        assert_eq!(
            rebind_failed["event_type_proto"],
            json!("REMOTE_DESKTOP_EVENT_TARGET_CHANGED")
        );
    }

    #[test]
    fn pending_media_rebind_candidate_failure_restores_active_session() {
        let mut session = RemoteDesktopSession::new(test_application_session_init(
            "rd-media-rebind-filter-failed",
            vec!["webrtc".into()],
        ));
        let epoch = TransportEpoch::new(21);
        let original_binding_epoch = session.target_binding().binding_epoch();
        let original_identity_epoch = session.target_binding().target_identity_epoch();
        let original_media_source_epoch = session.target_binding().media_source_epoch();

        session.begin_webrtc_negotiation(epoch);
        session
            .set_local_webrtc_answer(
                epoch,
                json!({"type": "answer", "sdp": "v=0", "media_scope": "video_only"}),
                "sck-native",
                true,
                "easynet:///r/acme/ability/remote-desktop.transport".into(),
            )
            .expect("local answer records");
        session.mark_webrtc_media_sending(
            epoch,
            direct_webrtc_endpoint_ura("rd-media-rebind-filter-failed"),
        );
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::MediaActive
        );

        assert!(
            session
                .record_target_observation(TargetObservation::ApplicationSurfaceChanged {
                    app_window_set: AppWindowSetProof::new(
                        42,
                        Some("com.example.Editor".to_string()),
                        Some(9001),
                        vec![10, 11, 12],
                    ),
                    app_surface_layout: None,
                    geometry: TargetGeometry {
                        x: Some(10.0),
                        y: Some(20.0),
                        width: Some(320.0),
                        height: Some(120.0),
                    },
                    target_identity_epoch: 100,
                    target_geometry_revision: 4,
                    observed_at_ms: 10,
                })
                .is_none(),
            "application window-set drift rebind must not be reported as media loss"
        );
        let pending = session
            .pending_media_rebind_binding()
            .expect("session exposes pending media rebind")
            .clone();
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::Rebinding
        );

        assert!(session.supersede_pending_media_rebind(
            epoch,
            TargetResolutionError::ScreenCaptureKitFilterFailed,
            "native content filter rejected pending application window set".to_string(),
        ));

        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::MediaActive
        );
        assert_eq!(session.state(), RemoteDesktopState::Negotiating);
        assert_eq!(
            session.target_tracking_state()["status"],
            json!("resolved"),
            "a stale candidate must restore the still-committed target"
        );
        assert!(session.pending_media_rebind_binding().is_none());
        let events = session.events();
        let rebind_superseded = events
            .iter()
            .find(|event| event["event_type"] == json!("TARGET_REBIND_SUPERSEDED"))
            .expect("stale candidate emits a non-terminal lifecycle event");
        assert_eq!(
            rebind_superseded["reason_code"],
            json!("target_rebind_candidate_superseded")
        );
        assert_eq!(
            rebind_superseded["payload"]["candidate_rejection_reason"],
            json!("screencapturekit_filter_failed")
        );
        assert_eq!(
            rebind_superseded["payload"]["recoverability"],
            json!("continue")
        );
        assert!(rebind_superseded["payload"]["frontend_action"].is_null());
        assert_eq!(
            rebind_superseded["binding_epoch"],
            json!(original_binding_epoch)
        );
        assert_eq!(
            rebind_superseded["target_identity_epoch"],
            json!(original_identity_epoch)
        );
        assert_eq!(
            rebind_superseded["media_source_epoch"],
            json!(original_media_source_epoch)
        );
        assert_eq!(
            rebind_superseded["payload"]["pending_binding_epoch"],
            json!(pending.binding_epoch())
        );
        assert_eq!(
            rebind_superseded["payload"]["pending_target_identity_epoch"],
            json!(pending.target_identity_epoch())
        );
        assert_eq!(
            rebind_superseded["payload"]["pending_media_source_epoch"],
            json!(pending.media_source_epoch())
        );
        assert_target_tracking_payload_context(rebind_superseded, &session);
        assert_eq!(
            rebind_superseded["payload"]["target_binding"]["binding_epoch"],
            json!(original_binding_epoch),
            "current binding context must not be overwritten by pending rebind evidence"
        );
        assert_eq!(
            rebind_superseded["payload"]["target_binding"]["media_source_epoch"],
            json!(original_media_source_epoch),
            "superseded media rebind keeps the committed media source context"
        );
        assert_eq!(
            rebind_superseded["event_type_proto"],
            json!("REMOTE_DESKTOP_EVENT_TARGET_CHANGED")
        );
        assert!(events
            .iter()
            .all(|event| event["event_type"] != json!("MEDIA_SOURCE_LOST")));
    }

    #[test]
    fn active_window_resize_atomically_updates_session_binding_and_capture_proof() {
        let session_id = "rd-window-resize-generation";
        let mut session =
            RemoteDesktopSession::new(test_window_session_init(session_id, vec!["webrtc".into()]));
        let epoch = TransportEpoch::new(31);
        let original_binding_epoch = session.target_binding().binding_epoch();
        let original_media_source_epoch = session.target_binding().media_source_epoch();
        let original_geometry_revision = session.target_binding().target_geometry_revision();

        session.begin_webrtc_negotiation(epoch);
        session
            .set_local_webrtc_answer(
                epoch,
                json!({"type": "answer", "sdp": "v=0", "media_scope": "video_only"}),
                "xcap-openh264-webrtc",
                true,
                direct_webrtc_endpoint_ura(session_id),
            )
            .expect("window WebRTC answer records");
        session.mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura(session_id));
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::MediaActive
        );

        assert!(
            session
                .record_target_observation(TargetObservation::GeometryChanged {
                    geometry: TargetGeometry {
                        x: Some(100.0),
                        y: Some(200.0),
                        width: Some(1024.0),
                        height: Some(720.0),
                    },
                    target_geometry_revision: original_geometry_revision + 1,
                    observed_at_ms: 4_000,
                })
                .is_none(),
            "resize is a generation transition, not media-source loss"
        );
        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::Rebinding
        );
        assert_eq!(
            session.target_binding().binding_epoch(),
            original_binding_epoch
        );
        let pending = session
            .pending_media_rebind_binding()
            .expect("window resize exposes pending generation")
            .clone();
        assert_eq!(pending.binding_epoch(), original_binding_epoch + 1);
        assert_eq!(
            pending.media_source_epoch(),
            original_media_source_epoch + 1
        );
        let capture_proof = pending
            .require_capture_proof("test.ability")
            .unwrap()
            .clone()
            .reverified_with_native_dimensions(Some((1024, 720)));

        assert!(session.commit_pending_media_rebind(
            epoch,
            pending.binding_epoch(),
            pending.media_source_epoch(),
            capture_proof,
        ));

        assert_eq!(
            session.lifecycle_phase(),
            RemoteDesktopSessionPhase::MediaActive
        );
        let view = serialize_session(&session);
        assert_eq!(
            view["target_binding"]["binding_epoch"],
            json!(original_binding_epoch + 1)
        );
        assert_eq!(
            view["target_binding"]["media_source_epoch"],
            json!(original_media_source_epoch + 1)
        );
        assert_eq!(view["target_binding"]["bounds"]["width"], json!(1024.0));
        assert_eq!(view["target_binding"]["bounds"]["height"], json!(720.0));
        assert_eq!(
            view["target_binding"]["capture_proof"]["native_width"],
            json!(1024)
        );
        assert_eq!(
            view["target_binding"]["capture_proof"]["native_height"],
            json!(720)
        );
        let events = session.events();
        let rebound = events
            .iter()
            .find(|event| {
                event["event_type"] == json!("TARGET_REBOUND")
                    && event["payload"]["detail"]
                        == json!("target_geometry_change_requires_media_source_rebuild")
            })
            .expect("resize commit emits target rebound");
        assert_eq!(
            rebound["payload"]["reason_code"],
            json!("target_geometry_media_source_rebound")
        );
    }
}
