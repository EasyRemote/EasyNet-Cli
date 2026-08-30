#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-lifecycle-input-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p "$SANDBOX/docs/design"
mkdir -p "$SANDBOX/plugins/remote-desktop/src/handlers"
mkdir -p "$SANDBOX/plugins/remote-desktop/src/input"
mkdir -p "$SANDBOX/plugins/remote-desktop/src/transport"
mkdir -p "$SANDBOX/plugins/remote-desktop/native-host/src"
mkdir -p "$SANDBOX/plugins/remote-desktop/media-host/src"

write_fixture() {
  rm -rf "$SANDBOX/docs" "$SANDBOX/plugins"
  mkdir -p "$SANDBOX/docs/design"
  mkdir -p "$SANDBOX/plugins/remote-desktop/src/handlers"
  mkdir -p "$SANDBOX/plugins/remote-desktop/src/input"
  mkdir -p "$SANDBOX/plugins/remote-desktop/src/transport"
  mkdir -p "$SANDBOX/plugins/remote-desktop/native-host/src"
  mkdir -p "$SANDBOX/plugins/remote-desktop/media-host/src"

  cat >"$SANDBOX/docs/design/remoteapp-targeted-session-spec.md" <<'MD'
| E2E-08 move/resize tracking | move and resize events advance target geometry revision and input consumes that revision |
| E2E-09 target loss vs transport failure | selected target loss emits target/media loss without transport failure |
| E2E-10 weak identity ambiguity | ambiguous weak native identity fails closed before stream start |
| E2E-11 view-only input safety | app/window sessions remain view-only without a focus-safe input validator |
| E2E-14 guarded target-local input | target-local input is allowed only after a fresh identity/focus/geometry guard proof |
relay_ready
Cross-display application window-set rebind is implemented through the explicit pending-media-rebind state machine and emits TARGET_REBOUND only after a renewed capture proof commits.
Direct WebRTC route discovery is provider-backed. Host candidates, configured STUN server-reflexive routes, standard TURN relay routes, and EasyNet relay routes are represented as typed route evidence.
Target-local input snapshot validation uses a 50 ms monotonic deadline.
MD

  cat >"$SANDBOX/plugins/remote-desktop/src/constants.rs" <<'RS'
pub const REASON_TARGET_PERMISSION_REVOKED: &str = "target_permission_revoked";

fn direct_webrtc_endpoint_ura(session_id: &str) -> String {
    format!(
        "easynet:///r/local/resource/remote-desktop-transport.{}/endpoint/webrtc",
        hex::encode(session_id.as_bytes())
    )
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/target_tracking.rs" <<'RS'
pub struct TargetTrackerSnapshot {
    pub target_geometry_revision: u64,
}

const TARGET_LIFECYCLE_EVENT_COALESCE_INTERVAL_MS: u64 = 100;
const AUTOMATIC_REBIND_WINDOW_MS: u64 = 30_000;

struct TargetLifecycleEventCoalescer;

fn commit_geometry() {
    geometry_event_types();
    ApplicationSurfaceChanged;
    "TARGET_PERMISSION_REVOKED";
    "TARGET_PERMISSION_VERIFICATION_PENDING";
    "TARGET_PERMISSION_VERIFICATION_CLEARED";
    PermissionVerificationPending;
    "target_title_after_loss";
    "target_focus_after_loss";
    "target_loss_pending";
    input_blocked_reason;
    payload["failure_domain"] = json!("target");
    target_failure_payload();
    FrontendAction::RetrySession;
    TargetResolutionError::TargetHidden;
    TargetResolutionError::TargetMinimized;
    TargetResolutionError::TargetDisplayUnavailable.frontend_action();
    "TARGET_BLURRED";
    if target_lost {
        "TARGET_REBIND_FAILED";
        "explicit_rebind_required";
        "DISPLAY_TOPOLOGY_CHANGED";
        json!({
            "target_display_unavailable": true,
            "target_hidden": true,
            "target_minimized": true,
            "target_blurred": true,
            "retry_session": true,
            "target_status": "lost",
            "input_enabled": false,
        });
    }
}

fn coalesced_lifecycle_event() {
    payload["coalesced_target_events"] = json!(0);
}

struct TargetTrackingEmission;

fn ordered_events() {}

fn input_blocked_reason() {}

fn target_failure_payload() {}

fn commit_pending_media_rebind() {
    "TARGET_REBOUND";
}

fn commit_pending_media_rebind_failed() {
    "TARGET_REBIND_FAILED";
}

fn commit_application_surface() {}

fn stage_application_surface_media_rebind() {}

fn expire_rebind_deadline() {
    "rebind_window_expired";
}

fn geometry_event_types() -> Vec<&'static str> {
    vec!["TARGET_MOVED", "TARGET_RESIZED"]
}

#[cfg(test)]
mod tests {
    #[test]
    fn tracker_commits_move_resize_and_lost_without_rebinding() {}

    #[test]
    fn tracker_expands_combined_move_resize_observation_into_ordered_events() {}

    #[test]
    fn tracker_coalesces_high_rate_geometry_and_title_events() {}

    #[test]
    fn tracker_debounces_single_transient_lost_observation() {
        assert_eq!(tracker.snapshot().to_value()["input_enabled"], json!(false));
        assert_eq!(tracker.snapshot().to_value()["input_blocked_reason"], json!("target_loss_pending"));
    }

    #[test]
    fn tracker_reports_rebind_failure_after_target_loss_without_policy() {
        assert_eq!(rebind_attempted.payload()["failure_domain"], json!("target"));
        assert_eq!(rebind_failed.payload()["failure_domain"], json!("target"));
        assert_eq!(tracker.snapshot().latest_diagnostic()["failure_domain"], json!("target"));
    }

    #[test]
    fn tracker_routes_post_loss_title_focus_through_explicit_rebind() {}

    #[test]
    fn active_application_window_set_rebind_failure_is_typed() {}

    #[test]
    fn active_application_z_order_change_rebuilds_media_without_changing_identity() {}

    #[test]
    fn pending_media_rebind_expires_at_rebind_deadline() {}

    #[test]
    fn post_loss_rebind_attempt_expires_at_rebind_deadline() {}

    #[test]
    fn display_topology_loss_projects_target_failure_recovery() {
        assert_eq!(topology_changed.payload()["reason_code"], json!("target_display_unavailable"));
        assert_eq!(topology_changed.payload()["input_blocked_reason"], json!("target_display_unavailable"));
        assert_eq!(hidden.payload()["reason_code"], json!("target_hidden"));
        assert_eq!(hidden.payload()["input_blocked_reason"], json!("target_hidden"));
        assert_eq!(minimized.payload()["reason_code"], json!("target_minimized"));
        assert_eq!(minimized.payload()["input_blocked_reason"], json!("target_minimized"));
        assert_eq!(blurred.payload()["reason_code"], json!("target_blurred"));
        assert_eq!(blurred.payload()["input_blocked_reason"], json!("target_blurred"));
        assert_eq!(hidden.payload()["frontend_action"], json!("retry_session"));
    }

    #[test]
    fn permission_verification_is_fail_closed_recoverable_and_durable() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session.rs" <<'RS'
struct RemoteDesktopSession {
    consent: RemoteDesktopConsentState,
    input_runtime_block_reason: Option<String>,
}

fn new() {
    RemoteDesktopConsentState::active(consent_grant, consent_epoch);
}

fn record_target_observation() {
    let target_loss_reason = match &observation {
        TargetObservation::Lost { reason, .. } => Some(*reason),
        TargetObservation::PermissionVerificationRequired { .. } => Some(TargetResolutionError::TargetPermissionMissing),
        TargetObservation::PermissionRevoked { .. } => Some(TargetResolutionError::TargetPermissionMissing),
        _ => None,
    };
    let input_was_enabled = self.target.snapshot().input_enabled();
    if input_was_enabled && !self.target.snapshot().input_enabled() {
        self.lifecycle.deactivate_input_for_target_block();
    }
    if target_loss_reason.is_some() {
        self.consent.revoke();
        self.lifecycle.suspend();
        media_source_lost = self.mark_active_media_source_lost(reason);
        session_events::media_source_lost(self.target.binding());
    }
    self.push_target_tracking_event(event);
    if permission_revoked {
        self.begin_close_after_permission_revoked();
    }
}

fn push_target_tracking_event() {
    payload["transport_epoch"] = self.transport.active_epoch();
    for (event_type, mut payload) in event.ordered_events() {}
}

fn push_projected_event(&mut self, event: session_events::RemoteDesktopEventProjection) {
    self.push_event(event.event_type(), event.into_payload());
}

fn report_client_media_state() {
    session_events::client_media_state_changed(state, epoch.value());
    session_events::session_degraded(state, epoch.value(), "degraded");
}

fn production_media_ready() -> bool {
    self.target.binding().production_scope_ready()
        && self.signaling.production_codec_negotiated()
        && self.signaling.production_backend_ready()
        && self.transport.media_transport_ready()
        && self.transport.client_media_ready()
}

fn activate_input_for_transport_epoch() {
    if !self.consent.permits_media_input() {
        return false;
    }
}

fn input_runtime_block_reason(&self) -> Option<&str> {
    self.input_runtime_block_reason.as_deref()
}

fn mark_input_permission_blocked() {
    self.input_runtime_block_reason = Some(reason.to_string());
    self.lifecycle.deactivate_input_for_runtime_block();
}

fn close() {
    self.consent.expire();
}

fn revoke_consent() {
    self.lifecycle.suspend();
    media_source_lost = self.mark_active_media_source_lost(reason);
}

fn begin_close_after_permission_revoked(&mut self) {
    self.lifecycle.begin_termination(REASON_TARGET_PERMISSION_REVOKED);
}

fn finish_close(&mut self, reason: &str) {
    self.lifecycle.terminate_closed(reason);
    self.terminal_receipt = Some(
        self.project_terminal_receipt(reason, &terminal_event),
    );
}

fn expire_target_rebind_deadline() {
    self.lifecycle.reject_rebinding();
    self.mark_active_media_source_lost(reason);
    self.push_target_tracking_event(event);
}

fn mark_active_media_source_lost() {
    self.transport.mark_media_source_lost(epoch);
}

fn begin_webrtc_negotiation() {
    self.signaling.begin_transport_generation();
}

fn mark_webrtc_generation_failed_with_context() {
    self.transport.mark_failed(epoch);
    self.lifecycle.suspend();
    session_events::webrtc_failed_with_context();
}

#[cfg(test)]
mod tests {
    #[test]
    fn target_lost_stops_active_media_source_without_transport_failure() {
        assert!(target_lost_index < media_source_lost_index);
        assert_eq!(session.state(), RemoteDesktopState::Suspended);
        assert_eq!(events[target_lost_index]["state_proto"], json!("REMOTE_DESKTOP_SESSION_STATE_SUSPENDED"));
        assert_eq!(events[target_lost_index]["payload"]["failure_domain"], json!("target"));
        assert_eq!(events[target_lost_index]["payload"]["frontend_action"], json!("refresh_targets"));
        assert_eq!(events[target_lost_index]["transport_epoch"], json!(epoch.value()));
        assert_eq!(events[media_source_lost_index]["binding_id"], json!(session.target_binding().binding_id()));
        assert_eq!(events[media_source_lost_index]["target_identity_epoch"], json!(session.target_binding().target_identity_epoch()));
        assert_eq!(events[media_source_lost_index]["media_source_epoch"], json!(session.target_binding().media_source_epoch()));
        assert_eq!(events[media_source_lost_index]["consent_epoch"], json!(session.target_binding().consent_epoch()));
        assert_eq!(session.target_tracking_state()["input_enabled"], json!(false));
        assert!(events.iter().all(|event| event["event_type"] != json!("SESSION_DEGRADED")));
    }

    #[test]
    fn target_tracking_events_include_active_transport_epoch_at_session_boundary() {
        assert_eq!(target_event["transport_epoch"], json!(epoch.value()));
        assert_eq!(target_event["payload"]["transport_epoch"], json!(epoch.value()));
        assert!(
            geometry_events[1]["sequence"].as_u64().unwrap()
                == geometry_events[0]["sequence"].as_u64().unwrap() + 1,
            "combined geometry observation must expand into monotonic ordered event-log rows"
        );
    }

    #[test]
    fn session_close_events_project_terminal_reason_code() {}

    let closing_index = 1;
    let closed_index = 2;
    assert!(closing_index < closed_index);
    let closing = event;
    assert_eq!(closing["recoverability"], json!("closing"));

    #[test]
    fn session_expiry_events_project_terminal_reason_code() {}

    #[test]
    fn initial_session_events_project_reason_codes_in_order() {
        assert!(created_index < resolved_index && resolved_index < bound_index);
        assert_eq!(created["reason_code"], json!("session_created"));
        assert_eq!(created["recoverability"], json!("continue"));
        assert_eq!(resolved["reason_code"], json!("capture_target_resolved"));
        assert_eq!(resolved["recoverability"], json!("continue"));
        assert_eq!(resolved["binding_id"], json!(session.target_binding().binding_id()));
    }

    #[test]
    fn pending_target_loss_deactivates_input_before_media_loss_debounce() {
        assert!(media_loss.is_none());
        assert_eq!(session.lifecycle_phase(), RemoteDesktopSessionPhase::MediaActive);
        assert_eq!(session.target_tracking_state()["input_blocked_reason"], json!("target_loss_pending"));
    }

    #[test]
    fn target_loss_rejects_late_client_media_state_without_degrading_session() {
        assert!(!session.report_client_media_state(epoch, "stalled", None));
        assert_eq!(session.state(), RemoteDesktopState::Suspended);
        assert_eq!(session.transport_state()["primary"], json!("media_source_lost"));
        assert_eq!(session.transport_state()["device_sending"], json!(false));
        assert!(events.iter().all(|event| event["event_type"] != json!("SESSION_DEGRADED")));
    }

    #[test]
    fn client_media_stall_emits_session_degraded_recovery_event() {
        assert!(session.report_client_media_state(epoch, "presenting", None));
        assert!(session.report_client_media_state(epoch, "stalled", None));
        assert_eq!(session.state(), RemoteDesktopState::Degraded);
        assert_eq!(session.transport_state()["primary"], json!("degraded"));
        assert!(!session.client_media_ready());
        assert!(!session.production_media_ready());
        let client_state_index = 1;
        let degraded_index = 2;
        assert!(client_state_index < degraded_index);
        let degraded = event;
        assert_eq!(degraded["reason_code"], json!("client_media_stalled"));
        assert_eq!(degraded["recoverability"], json!("retry_session"));
        assert_eq!(degraded["payload"]["primary_phase"], json!("degraded"));
        assert_eq!(degraded["payload"]["frontend_action"], json!("retry_session"));
    }

    #[test]
    fn rehydrated_non_terminal_session_preserves_runtime_input_block_reason() {
        assert_eq!(
            session.input_runtime_block_reason(),
            Some("accessibility_permission_denied")
        );
    }

    #[test]
    fn runtime_input_permission_block_deactivates_input_without_failing_media() {
        assert!(session.media_transport_ready());
        assert!(!session.input_readiness()["interactive_ready"].as_bool().unwrap());
    }

    #[test]
    fn target_reappearance_after_loss_emits_explicit_rebind_failure() {
        let rebind_attempted = event;
        assert_eq!(rebind_attempted["reason_code"], json!("target_rebind_attempted"));
        assert_eq!(rebind_attempted["recoverability"], json!("retry_session"));
        assert_eq!(rebind_attempted["binding_id"], json!(session.target_binding().binding_id()));
        assert_eq!(rebind_attempted["target_identity_epoch"], json!(session.target_binding().target_identity_epoch()));
        assert_eq!(rebind_attempted["media_source_epoch"], json!(session.target_binding().media_source_epoch()));
        let rebind_failed = event;
        assert_eq!(rebind_failed["event_type"], json!("TARGET_REBIND_FAILED"));
        assert_eq!(rebind_failed["reason_code"], json!("explicit_rebind_required"));
        assert_eq!(rebind_failed["recoverability"], json!("new_session_required"));
        assert_eq!(rebind_failed["binding_id"], json!(session.target_binding().binding_id()));
        assert_eq!(rebind_failed["target_identity_epoch"], json!(session.target_binding().target_identity_epoch()));
        assert_eq!(rebind_failed["media_source_epoch"], json!(session.target_binding().media_source_epoch()));
    }

    #[test]
    fn target_rebind_deadline_expiry_rejects_session_rebinding() {}

    #[test]
    fn pending_media_rebind_failure_rejects_session_rebinding() {}

    #[test]
    fn pending_media_rebind_candidate_failure_restores_active_session() {}

    #[test]
    fn production_media_ready_requires_target_scope_ready() {
        assert!(
            !session.production_media_ready(),
            "scope widening or display fallback must prevent production online"
        );
        assert_eq!(target_bound["payload"]["display_fallback_used"], json!(true));
        assert_eq!(target_bound["consent_epoch"], json!(session.target_binding().consent_epoch()));
    }

    #[test]
fn consent_revocation_terminates_session_and_blocks_input_activation() {
        session.finish_close(REASON_TARGET_PERMISSION_REVOKED);
        assert!(permission_revoked_index < media_source_lost_index);
        assert!(media_source_lost_index < session_closed_index);
        assert_eq!(
            session.terminal_receipt().unwrap()["reason_code"],
            json!(REASON_TARGET_PERMISSION_REVOKED)
        );
        assert!(
            !session.activate_input_for_transport_epoch(epoch),
            "revoked consent must prevent input from reactivating even with the same transport epoch"
        );
    }
}

fn first_permission_denial_suspends_media_without_revoking_consent_or_session() {}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_recovery.rs" <<'RS'
struct RemoteDesktopRecoverySnapshot {
    #[serde(default)]
    transport_epoch_high_watermark: u64,
    #[serde(default)]
    input_runtime_block_reason: Option<String>,
}

impl RemoteDesktopRecoverySnapshot {
    fn input_runtime_block_reason(&self) -> Option<&str> {
        self.input_runtime_block_reason.as_deref()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn recovery_snapshot_round_trips_runtime_input_block_reason() {}

    #[test]
    fn recovery_snapshot_keeps_legacy_rows_without_runtime_input_block_reason_loadable() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_consent_state.rs" <<'RS'
enum RemoteDesktopConsentPhase {
    Active,
    Revoked,
    Expired,
}

impl RemoteDesktopConsentPhase {
    fn permits_media_input(self) -> bool {
        matches!(self, Self::Active)
    }
}

struct RemoteDesktopConsentState {
    phase: RemoteDesktopConsentPhase,
}

impl RemoteDesktopConsentState {
    fn active() -> Self {
        Self {
            phase: RemoteDesktopConsentPhase::Active,
        }
    }

    fn permits_media_input(&self) -> bool {
        self.phase.permits_media_input()
    }

    fn revoke(&mut self) {
        self.phase = RemoteDesktopConsentPhase::Revoked;
    }

    fn expire(&mut self) {
        self.phase = RemoteDesktopConsentPhase::Expired;
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_identity.rs" <<'RS'
struct RemoteDesktopSessionInit {
    consent: RemoteDesktopConsentGrant,
}

struct RemoteDesktopSessionProfile {
    session_id: String,
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/contract.rs" <<'RS'
enum RemoteDesktopSessionState {
    Suspended,
}

impl RemoteDesktopSessionState {
    fn wire_name(self) -> &'static str {
        match self {
            Self::Suspended => "REMOTE_DESKTOP_SESSION_STATE_SUSPENDED",
        }
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_state.rs" <<'RS'
fn suspend() {
    self.set_non_terminal_state(RemoteDesktopState::Suspended);
}

fn deactivate_input_for_runtime_block() {
    self.input_activation = InputActivationGate::RuntimePermissionBlocked;
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_transport_state.rs" <<'RS'
struct RemoteDesktopTransportState {
    epoch_high_watermark: u64,
}

enum PrimaryMediaPhase {
    MediaSourceLost,
    Failed,
}

fn can_transition_primary() {
    match from {
        PrimaryMediaPhase::MediaSourceLost => matches!(to, PrimaryMediaPhase::Failed),
    }
}

fn begin_primary() {
    if epoch.value() <= self.epoch_high_watermark {
        return false;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn media_source_lost_is_absorbing_until_new_epoch_or_failure() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_events.rs" <<'RS'
struct RemoteDesktopEventProjection {
    event_type: &'static str,
    payload: Value,
}

impl RemoteDesktopEventProjection {
    fn new(event_type: &'static str, payload: Value) -> Self {
        Self { event_type, payload }
    }

    fn event_type(&self) -> &'static str {
        self.event_type
    }

    fn into_payload(self) -> Value {
        self.payload
    }
}

enum WebRtcFailureEventKind {
    MediaSourceLost,
    TransportFailed,
}

impl WebRtcFailureEventKind {
    fn event_type(self) -> &'static str {
        match self {
            Self::MediaSourceLost => "MEDIA_SOURCE_LOST",
            Self::TransportFailed => "TRANSPORT_FAILED",
        }
    }
}

fn webrtc_transport_failure_context() {
    json!({
        "reason_code": TargetResolutionError::TransportRouteUnavailable.as_str(),
        "recoverability": "retry_session",
        "failure_domain": "transport",
        "frontend_action": FrontendAction::RetrySession.as_str(),
    });
}

fn session_created() {
    json!({
        "transport_kind": TRANSPORT_WEBRTC,
        "media_transport_ready": false,
        "preview_ability": ABILITY_ATTACH_SESSION,
        "reason_code": "session_created",
        "recoverability": "continue",
    });
}

fn capture_target_resolved(binding: &RemoteAppTargetBinding) {
    json!({
        "subject_ura": binding.subject_ura(),
        "binding_id": binding.binding_id(),
        "binding_epoch": binding.binding_epoch(),
        "previous_target_identity_epoch": Value::Null,
        "target_identity_epoch": binding.target_identity_epoch(),
        "target_geometry_revision": binding.target_geometry_revision(),
        "media_source_epoch": binding.media_source_epoch(),
        "consent_epoch": binding.consent_epoch(),
        "reason_code": "capture_target_resolved",
        "recoverability": "continue",
        "target_binding": binding.to_value(),
        "scope_audit": binding.scope_audit_value(),
    });
}

#[test]
fn capture_target_resolved_payload_projects_initial_binding_context() {
    assert_eq!(payload["consent_epoch"], json!(binding.consent_epoch()));
}

fn session_closed(reason: &str) {
    json!({
        "reason": reason,
        "reason_code": reason,
        "recoverability": "closed",
    });
}

fn session_closing(reason: &str) {
    json!({
        "reason": reason,
        "reason_code": reason,
        "recoverability": "closing",
    });
}

fn session_expired(reason: &str) {
    json!({
        "reason": reason,
        "reason_code": reason,
        "recoverability": "closed",
    });
}

fn media_source_lost(binding: &RemoteAppTargetBinding) {
    json!({
        "event_type": "MEDIA_SOURCE_LOST",
        "subject_ura": binding.subject_ura(),
        "binding_id": binding.binding_id(),
        "binding_epoch": binding.binding_epoch(),
        "target_identity_epoch": binding.target_identity_epoch(),
        "target_geometry_revision": binding.target_geometry_revision(),
        "media_source_epoch": binding.media_source_epoch(),
        "consent_epoch": binding.consent_epoch(),
        "failure_domain": "target",
        "media_transport_ready": false,
    });
}

fn client_media_reason_code(state: &str) -> &'static str {
    match state {
        "presenting" => "client_media_presenting",
        "detached" => "client_media_detached",
        _ => "client_media_stalled",
    }
}

fn client_media_state_changed(state: &str) {
    json!({
        "reason_code": client_media_reason_code(state),
        "recoverability": if state == "presenting" { "continue" } else { "retry_session" },
    });
}

fn session_degraded(client_state: &str, transport_epoch: u64, primary_phase: &str) {
    json!({
        "event_type": "SESSION_DEGRADED",
        "reason_code": client_media_reason_code(client_state),
        "recoverability": "retry_session",
        "failure_domain": "client_media",
        "frontend_action": FrontendAction::RetrySession.as_str(),
        "transport_kind": TRANSPORT_WEBRTC,
        "transport_epoch": transport_epoch,
        "primary_phase": primary_phase,
        "client_media_ready": false,
    });
}

fn input_permission_blocked() {
    RemoteDesktopEventProjection::new(
        "INPUT_PERMISSION_BLOCKED",
        json!({
            "recoverability": "request_input_permission",
            "frontend_action": FrontendAction::RequestPermission.as_str(),
        }),
    );
}

fn input_permission_restored() {
    RemoteDesktopEventProjection::new(
        "INPUT_PERMISSION_RESTORED",
        json!({
            "recoverability": "resolved",
        }),
    );
}

fn transport_blocked() {
    let blocker = RemoteDesktopTransportBlocker::from_webrtc_error(reason);
    json!({
        "reason_code": blocker.map(RemoteDesktopTransportBlocker::reason_code_str),
        "frontend_action": blocker
            .map(RemoteDesktopTransportBlocker::frontend_action)
            .map(|action| action.as_str()),
    });
}

#[test]
fn transport_blocked_projects_capture_backend_reason_code() {}

#[test]
fn session_created_payload_projects_initial_reason_code() {}

#[test]
fn capture_target_resolved_payload_projects_initial_binding_context() {}

#[test]
fn session_closing_payload_projects_terminal_reason_code() {}

#[test]
fn session_closed_payload_projects_terminal_reason_code() {}

#[test]
fn session_expired_payload_projects_terminal_reason_code() {}

#[test]
fn session_degraded_payload_projects_recovery_context() {}

#[test]
fn input_permission_block_projects_request_permission_recovery() {}

#[test]
fn input_permission_restore_projects_resolved_recovery() {}

#[test]
fn session_created_projects_remote_desktop_attach_as_preview_ability() {}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_media.rs" <<'RS'
fn direct_webrtc_target_failure_projection() {
    WebRtcFailureEventKind::MediaSourceLost;
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/event_log.rs" <<'RS'
const TARGET_CHANGED_EVENT_TYPES: &[&str] = &[
    "CAPTURE_TARGET_STALE",
    "CAPTURE_TARGET_IDENTITY_MISMATCH",
    "CAPTURE_TARGET_AMBIGUOUS",
    "DISPLAY_FALLBACK_FORBIDDEN",
    "SCREEN_CAPTURE_PERMISSION_DENIED",
    "TARGET_MOVED",
    "TARGET_RESIZED",
    "TARGET_TITLE_CHANGED",
    "TARGET_FOCUSED",
    "TARGET_BLURRED",
    "TARGET_HIDDEN",
    "TARGET_VISIBLE",
    "TARGET_MINIMIZED",
    "TARGET_RESTORED",
    "TARGET_LOST",
    "TARGET_REBIND_ATTEMPTED",
    "TARGET_REBOUND",
    "TARGET_REBIND_FAILED",
    "TARGET_BINDING_CHANGED",
    "TARGET_PERMISSION_REVOKED",
    "DISPLAY_TOPOLOGY_CHANGED",
];

fn event_type_proto_name(event_type: &str) -> &'static str {
    let target_field = |name: &str| payload.get(name).cloned().unwrap_or(Value::Null);
    let consent_epoch = target_field("consent_epoch");
    json!({
        "consent_epoch": consent_epoch,
    });
    json!({
        "consent_epoch": Value::Null,
    });
    if TARGET_CHANGED_EVENT_TYPES.contains(&event_type) {
        return "REMOTE_DESKTOP_EVENT_TARGET_CHANGED";
    }
    match event_type {
        "INPUT_PERMISSION_BLOCKED" | "INPUT_PERMISSION_RESTORED" => "REMOTE_DESKTOP_EVENT_INPUT",
        _ => "REMOTE_DESKTOP_EVENT_STATE_CHANGED",
    }
}

#[test]
fn spec_target_lifecycle_events_have_explicit_proto_projection() {}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/transport_blocker.rs" <<'RS'
struct RemoteDesktopTransportBlocker;

impl RemoteDesktopTransportBlocker {
    fn from_webrtc_error() {
        TargetResolutionError::CaptureBackendUnavailable;
        TargetResolutionError::ScreenCaptureKitStreamStartFailed;
    }
}

#[test]
fn backend_unavailable_maps_to_capture_backend_unavailable() {}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/view_transport.rs" <<'RS'
fn transport_route_state() {
    struct RemoteDesktopTransportReadinessBlocker;
    fn summary() {
        "message": self.message,
        "reason_code": self.reason_code.clone(),
        "readiness_blocker": self.readiness_blocker(),
        "metadata": {
            "input_channel_label": INPUT_DATA_CHANNEL_LABEL,
            "reason_code": self.reason_code.clone(),
            "readiness_blocker": self.readiness_blocker(),
        };
    }
    json!({
        "host_candidate": true,
        "stun_srflx": false,
        "turn_relay": false,
        "easynet_relay": false,
        "failed": false,
        "production_ready": self.production_ready(session),
        "production_route_ready": self.production_route_ready(),
    });
    fn transport_readiness_blocker() {
        TargetResolutionError::TransportRouteUnavailable;
        "transport_route_unavailable";
        RemoteDesktopTransportBlocker::from_webrtc_error;
    }
    fn transport_route_failed() {}
    fn candidate_declares_easynet_relay() {}
    "host_only_no_nat_or_relay";
    "relay_unavailable";
}

fn direct_endpoint_ura(session: &RemoteDesktopSession) {
    direct_webrtc_endpoint_ura(session.session_id());
}

fn diagnostic_preview_summary() {
    json!({
        "preview_ability": ABILITY_ATTACH_SESSION,
    });
}

#[test]
fn host_only_candidates_are_not_reported_as_nat_or_relay_ready() {}

#[test]
fn host_only_route_keeps_production_offline_after_client_media_presents() {}

#[test]
fn easynet_relay_does_not_imply_turn_relay() {}

#[test]
fn turn_relay_hostname_containing_easynet_is_not_easynet_relay() {}

#[test]
fn srflx_without_relay_reports_typed_relay_unavailable_reason() {
    assert_eq!(summary["reason_code"], json!("transport_route_unavailable"));
}

#[test]
fn transport_summary_projects_remote_desktop_attach_as_preview_ability() {}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/network.rs" <<'RS'
enum DirectWebRtcRouteCandidateClass {
    Host,
    StunServerReflexive,
    TurnRelay,
    EasyNetRelay,
}

const DIRECT_WEBRTC_ROUTE_MODEL: &[DirectWebRtcRouteCandidateClass] = &[
    DirectWebRtcRouteCandidateClass::Host,
    DirectWebRtcRouteCandidateClass::StunServerReflexive,
    DirectWebRtcRouteCandidateClass::TurnRelay,
    DirectWebRtcRouteCandidateClass::EasyNetRelay,
];

impl DirectWebRtcRouteCandidateClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host_candidate",
            Self::StunServerReflexive => "stun_srflx",
            Self::TurnRelay => "turn_relay",
            Self::EasyNetRelay => "easynet_relay",
        }
    }
}

struct DirectWebRtcRouteCandidate;

impl DirectWebRtcRouteCandidate {
    fn endpoint(&self) -> &str {
        "127.0.0.1:0"
    }

    fn local_bind_endpoint(&self) -> Option<&str> {
        Some(self.endpoint())
    }
}

struct DirectWebRtcIceServerConfig;

struct DirectWebRtcRouteConfig;

trait DirectWebRtcRouteCandidateProvider {
    fn route_candidates(&self) -> Vec<DirectWebRtcRouteCandidate>;
}

struct LocalInterfaceRouteCandidateProvider;

struct ConfiguredDirectWebRtcRouteProvider;

impl ConfiguredDirectWebRtcRouteProvider {
    fn from_env() -> Self {
        Self
    }
}

#[test]
fn route_candidate_evidence_keeps_host_only_provider_explicit() {
    assert_eq!(evidence["provider_state"], json!("host_local_only"));
}

#[test]
fn configured_route_provider_projects_ice_servers_without_credentials_in_evidence() {
    assert_eq!(evidence["route_config"]["ice_servers"][0]["credential_configured"], json!(true));
}

#[test]
fn configured_ice_routes_do_not_become_local_udp_bind_endpoints() {}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_store.rs" <<'RS'
fn mark_direct_webrtc_media_ready(session_id: &str) {
    direct_webrtc_endpoint_ura(session_id);
}

fn mark_direct_webrtc_generation_failed() {
    WebRtcFailureEventKind::TransportFailed;
    webrtc_transport_failure_context();
}

fn fail_pending_media_rebind_for_session() {}

fn supersede_pending_media_rebind_for_session() {}

fn expire_target_rebind_deadline_for_session() {}

#[cfg(test)]
mod tests {
    #[test]
    fn production_media_ready_requires_production_codec_and_sender_ready() {
        assert_eq!(view["production_readiness"]["blocked_reason"], json!("production_backend_not_ready"));
        assert_eq!(view["production_readiness"]["client_media_ready"], json!(false));
        assert!(session.report_client_media_state(TransportEpoch::new(1), "presenting", None));
        assert_eq!(view["production_readiness"]["blocked_reason"], json!("production_route_not_ready"));
        assert_eq!(view["transport"]["production_ready"], json!(false));
        assert_eq!(view["transports"][0]["metadata"]["production_ready"], json!(false));
    }

    #[test]
    fn direct_webrtc_transport_failure_suspends_session_for_a_new_generation() {
        assert_eq!(event["reason_code"], json!("transport_route_unavailable"));
        assert_eq!(event["recoverability"], json!("retry_session"));
        assert_eq!(event["payload"]["failure_domain"], json!("transport"));
        assert_eq!(event["payload"]["frontend_action"], json!("retry_session"));
    }

    #[test]
    fn session_store_expires_target_rebind_deadline_for_bound_session() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs" <<'RS'
fn apply_pending_media_rebind() {
    sessions.supersede_pending_media_rebind_for_session(
        session_id,
        epoch,
        attempt_token,
    );
    generation = restart_generation(
        execution,
        &active_binding,
    );
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_endpoint.rs" <<'RS'
struct RTCIceServer;

fn answer(endpoint_config: EndpointConfig) {
    let route_candidate_provider = ConfiguredDirectWebRtcRouteProvider::from_env_with_relay_lease(relay_lease);
    let route_candidates = route_candidate_provider.route_candidates();
    let ice_servers = vec![RTCIceServer];
    RTCConfigurationBuilder::new().with_ice_servers(ice_servers);
    let udp_addrs = route_candidates
        .iter()
        .filter_map(|candidate| candidate.local_bind_endpoint().map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    json!({
        "endpoint_ura": direct_webrtc_endpoint_ura(&endpoint_config.session_id),
        "route_candidate_evidence": route_candidate_evidence,
    });
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_negotiation.rs" <<'RS'
fn mark_backend_unavailable() {
    session.mark_transport_blocked(
        "webrtc_transport_backend_unavailable",
        MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID,
    );
}

fn commit_started_endpoint(session_id: &str) {
    direct_webrtc_endpoint_ura(session_id);
}

fn begin_generation() {
    plugin.persist_recovery_snapshot(&recovery_snapshot);
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/handlers/set_description.rs" <<'RS'
#[test]
fn remote_offer_backend_gate_blocks_without_committing_signaling() {
    assert_eq!(signaled["signaling"]["remote_description"], Value::Null);
    assert_eq!(signaled["signaling"]["local_description"], Value::Null);
    assert!(
        signaled["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["event_type"] != json!("DESCRIPTION_SET"))
    );
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/view.rs" <<'RS'
fn serialize_session() {
    let transport_route_state = transport_view.route_state();
    let input_readiness = input_readiness_view(session, &effective_input_policy);
    let video_ready = transport_view.production_ready(session);
    let negotiated_media_scope = session.negotiated_media_scope();
    let audio_required = negotiated_media_scope.is_some_and(|scope| scope.requires_audio());
    let audio_ready = session.media_stats()["audio_ready"].as_bool().unwrap_or(false);
    let negotiated_media_scope_ready = negotiated_media_scope.is_some();
    let client_decode_ready = session.client_decode_ready();
    let ready = video_ready
        && negotiated_media_scope_ready
        && client_decode_ready
        && (!audio_required || audio_ready);
    json!({
        "consent": session.consent_state().to_value(),
        "input_readiness": input_readiness.clone(),
        "input_plane": {
            "readiness": input_readiness,
        },
        "signaling": {
            "route_state": transport_route_state.clone(),
        },
        "production_readiness": {
            "ready": ready,
            "blocked_reason": production_readiness_blocked_reason(
                session,
                transport_view,
                audio_ready,
                &Value::Null,
            ),
            "target_scope_ready": session.target_scope_ready(),
            "production_backend_ready": session.production_backend_ready(),
            "production_route_ready": transport_view.production_route_ready(),
            "route_readiness_blocker": transport_view.readiness_blocker(),
            "route_state": transport_route_state.clone(),
        },
    });
}

fn production_readiness_blocked_reason(session: &RemoteDesktopSession, transport_view: &RemoteDesktopTransportView, audio_ready: bool, audio_blocked_reason: &Value) -> Value {
    if transport_view.production_ready(session) {
        Value::Null
    } else if !session.target_scope_ready() {
        json!("target_scope_not_ready")
    } else if !session.production_codec_negotiated() {
        json!("production_codec_not_negotiated")
    } else if !session.production_backend_ready() {
        json!("production_backend_not_ready")
    } else if !negotiated_media_scope_ready {
        json!("media_scope_not_negotiated")
    } else if !session.media_transport_ready() {
        json!("media_transport_not_ready")
    } else if !session.client_media_ready() {
        json!("client_media_not_presenting")
    } else if !transport_view.production_route_ready() {
        json!("production_route_not_ready")
    } else {
        json!("production_readiness_incomplete")
    }
}

fn video_only_negotiation_requires_bound_decode_but_not_audio_runtime_stats() {}

fn audio_video_negotiation_requires_live_audio_runtime_stats() {}

fn input_readiness_view(session: &RemoteDesktopSession, input_policy: &EffectiveRemoteDesktopInputPolicy) -> Value {
    let blocked_reason = if let Some(reason) = session.input_runtime_block_reason() {
        json!(reason)
    } else {
        json!(session.target_binding().input_scope_reason())
    };
    let interactive_ready = false;
    json!({
        "requested_mode": session.mode(),
        "effective_mode": if interactive_ready { "interactive" } else { "view_only" },
        "interactive_ready": interactive_ready,
        "blocked_reason": blocked_reason,
        "input_scope": input_policy.input_scope().as_str(),
    })
}

fn session_view_projects_effective_view_only_input_scope() {
    assert_eq!(view["input_readiness"]["requested_mode"], json!("interactive"));
    assert_eq!(view["input_readiness"]["effective_mode"], json!("view_only"));
    assert_eq!(view["input_readiness"]["blocked_reason"], json!("target_scoped_keyboard_pointer_dispatch_unsafe"));
}

fn session_view_projects_session_local_runtime_input_blocker() {
    assert_eq!(view["input_readiness"]["blocked_reason"], json!("accessibility_permission_denied"));
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/view_device.rs" <<'RS'
fn device_capabilities_view() {
    let production_backend = native_webrtc_backend_runtime_descriptor();
    let production_ready = production_backend.production_ready();
    let production_target_subjects = if production_ready {
        production_backend.supported_subjects_value()
    } else {
        json!([])
    };
    let diagnostic_target_subjects = XCAP_OPENH264_WEBRTC_BACKEND.supported_subjects_value();
    let platform_support = platform_support_view(production_ready, &production_backend);
    let input_available = input_injection_available();
    let target_local_guard_available = target_scoped_input_guard_available();
    let input_control_support = input_control_support_view(input_available, target_local_guard_available);
    json!({
        "unsupported_input_types": unsupported_input_channel_types_value(),
        "unsupported_capabilities": [
            {
                "capability": "clipboard",
                "future_abilities": ["remote_desktop.clipboard.write"]
            },
            {
                "capability": "file_transfer",
                "future_abilities": ["remote_desktop.file_transfer.send"]
            }
        ],
        "metadata": {
            "production_target_subjects": production_target_subjects,
            "diagnostic_target_subjects": diagnostic_target_subjects,
            "production_target_subjects_source": if production_ready {
                MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID
            } else {
                "none"
            },
            "production_target_subjects_blocked_reason": if production_ready {
                Value::Null
            } else {
                json!(production_backend.unavailable_reason().unwrap_or("production_backend_not_ready"))
            },
            "platform_support": platform_support,
            "input_control_support": input_control_support,
            "capture_target_models": [
                "display_surface",
                "window_surface",
                "multi_surface_application_window_set",
                "process_scoped_application_window_set"
            ],
            "reason": "native ScreenCaptureKit/VideoToolbox WebRTC backend is available for display/window/application target capture"
        },
    });
}

fn input_control_support_view(input_available: bool, target_local_guard_available: bool) {
    let runtime_backend = input_injection_backend();
    let runtime_blocked_reason = input_injection_unavailable_reason();
    json!({
        "runtime_backend": runtime_backend,
        "runtime_blocked_reason": runtime_blocked_reason,
        "target_local_guard_compiled": target_local_guard_available,
        "target_local_runtime_available": input_available && target_local_guard_available,
        "certification": "live_e2e_required",
        "requires_input_control_consent": true,
        "input_transport": "webrtc_data_channel",
        "platforms": {
            "macos": {
                "display": {"status": "available", "scope": "display_global"},
                "window": {"status": "available", "reason": "macos_target_input_guard_ready", "scope": "target_local"},
                "application": {"status": "available", "reason": "macos_target_input_guard_ready", "scope": "target_local"}
            },
            "linux": {
                "display": {"status": "x11_display_global_ready", "reason": "linux_x11_xcb_atomic_display_global_ready"},
                "window": {"status": "view_only_only", "reason": "linux_x11_xtest_cannot_isolate_press_release_to_target", "scope": "view_only"},
                "application": {"status": "view_only_only", "reason": "linux_x11_xtest_cannot_isolate_press_release_to_target", "scope": "view_only"}
            },
            "windows": {
                "display": {"status": "baseline_ready", "reason": "windows_sendinput_target_guard_ready"},
                "window": {"status": "baseline_ready", "reason": "windows_sendinput_target_guard_ready"},
                "application": {"status": "baseline_ready", "reason": "windows_sendinput_target_guard_ready"}
            }
        }
    });
}

fn platform_support_view(production_ready: bool, production_backend: &Backend) {
    let macos_application = application_target_support(
        "production_ready",
        json!("macos.screencapturekit.videotoolbox.webrtc.v1"),
        "macos_screencapturekit_videotoolbox_ready",
        "multi_surface",
        true,
        None,
    );
    let process_application = application_target_support(
        "baseline_ready",
        json!("builtin.xcap.openh264.webrtc.v1"),
        "xcap_target_baseline_ready",
        "process_scoped",
        true,
        None,
    );
    json!({
        "application_surface": [macos_application, process_application],
        "platforms": {
            "linux": {
                "display": {"status": "baseline_ready", "reason": "linux_xcap_target_baseline_ready"},
                "window": {"status": "baseline_ready", "reason": "linux_xcap_target_baseline_ready"},
                "application": {"status": "baseline_ready", "reason": "linux_xcap_target_baseline_ready"}
            },
            "windows": {
                "display": {"status": "baseline_ready", "reason": "windows_xcap_target_baseline_ready"},
                "window": {"status": "baseline_ready", "reason": "windows_xcap_target_baseline_ready"},
                "application": {"status": "baseline_ready", "reason": "windows_xcap_target_baseline_ready"}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn device_capabilities_report_clipboard_and_file_transfer_unsupported() {}

    #[test]
    fn device_capabilities_project_native_target_subject_matrix() {}

    #[test]
    fn device_capabilities_project_cross_platform_support_matrix() {}

    #[test]
    fn device_capabilities_project_input_control_support_matrix() {}

    #[test]
    fn input_capability_keeps_display_global_but_blocks_target_local_without_guard() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/input.rs" <<'RS'
const UNSUPPORTED_INPUT_CHANNEL_TYPES: &[&str] = &["clipboard", "file_drop"];
const TARGET_INPUT_GUARD_PROVIDER_DEADLINE: Duration = Duration::from_millis(50);

fn unsupported_input_channel_types_value() {}

struct PointerInputFrame {
    target_geometry_revision: Option<u64>,
    sent_at_ms: Option<u64>,
    client_sequence: Option<u64>,
}

struct KeyInputFrame {
    sent_at_ms: Option<u64>,
    client_sequence: Option<u64>,
}

struct RemoteDesktopInputPolicy;

impl RemoteDesktopInputPolicy {
    fn to_value(&self) {
        json!({
            "unsupported_input_types": unsupported_input_channel_types_value(),
        });
    }
}

fn pointer_target_from_snapshot() {
    let _ = policy["pointer_target"]["target_geometry_revision"];
}

fn input_policy_object() {}

struct EffectiveRemoteDesktopInputPolicy {
    keyboard_enabled: bool,
    pointer_enabled: bool,
    input_scope: InputScope,
}

impl EffectiveRemoteDesktopInputPolicy {
    fn for_target_state() {}

    fn apply_scope(&mut self, input_scope: InputScope) {
        match input_scope {
            InputScope::ViewOnly => {
                self.keyboard_enabled = false;
                self.pointer_enabled = false;
            }
        }
    }

    fn reject_reason(&self, frame_type: &str) -> Option<&'static str> {
        if self.input_scope == InputScope::ViewOnly && matches!(frame_type, "key" | "pointer") {
            return Some("input_scope_unsupported");
        }
        None
    }
}

enum InputTransportGuard {
    DirectWebRtc(TransportEpoch),
    DiagnosticPreview,
}

fn current_session_input_policy() {
    current_session_effective_input_policy();
}

fn current_session_effective_input_policy() {
    InputTransportGuard::DirectWebRtc(epoch);
    let snapshot = session.target_snapshot();
    let binding = session.target_binding();
    if !snapshot.input_enabled() {
        return None;
    }
    policy.target_binding = Some(binding.clone());
    base_policy.for_current_target(snapshot, binding);
}

fn target_input_guard_validation() {
    let sample = executor.sample_for_input(TARGET_INPUT_GUARD_PROVIDER_DEADLINE)?;
    validate_target_pointer_input_observation(
        sample.observation(),
        binding,
        snapshot,
        point.x,
        point.y,
    );
}

fn target_snapshot_error_reason() {
    "target_input_guard_deadline_exceeded";
}

fn display_interactive_without_input_consent_remains_view_only() {}

fn input_policy_reject_reason() -> Option<&'static str> {
    if input_scope == Some(InputScope::ViewOnly.as_str()) && matches!(frame_type, "key" | "pointer") {
        return Some("input_scope_unsupported");
    }
    None
}

fn reject_unsupported_input_channel_frame() {}

fn validate_input_frame() {
    reject_unsupported_input_channel_frame(frame)?;
    validate_client_sent_at_ms(sent_at_ms)?;
    validate_client_sequence(client_sequence)?;
}

fn data_channel_loop() {
    if let Some(reason) = sequence_gate.reject_reason(client_sequence) {
        return InputApplyOutcome::rejected(reason);
    }
    let outcome = apply_input_frame_with_effective_policy(&effective_input_policy, &frame);
    if outcome.applied {
        sessions.mark_input_frame_applied(&session_id, epoch);
    }
    let reason = outcome.reason.unwrap_or("input_injection_failed");
    if input_runtime_permission_denied(reason) {
        sessions.mark_input_permission_blocked(&session_id, epoch, reason);
    }
}

#[test]
fn target_local_input_provider_hang_rejects_with_bounded_deadline() {}

const MAX_CLIENT_SENT_AT_MS: u64 = 9_007_199_254_740_991;
const MAX_CLIENT_SEQUENCE: u64 = 9_007_199_254_740_991;

fn validate_client_sent_at_ms() {}
fn validate_client_sequence() {}

struct InputFrameTiming {
    client_sent_at_ms: Option<u64>,
    host_received_at_ms: u64,
}

impl InputFrameTiming {
    fn latency_ms_at(&self, host_applied_at_ms: u64) -> Option<u64> {
        Some(host_applied_at_ms.saturating_sub(self.host_received_at_ms))
    }
}

struct InputSequenceGate;

impl InputSequenceGate {
    fn reject_reason(&mut self, client_sequence: Option<u64>) -> Option<&'static str> {
        if client_sequence == Some(1) {
            Some("stale_client_sequence")
        } else {
            None
        }
    }
}

impl RemoteDesktopInputFrame {
    fn client_sent_at_ms(&self) -> Option<u64> {
        Some(sent_at_ms)
    }

    fn client_sequence(&self) -> Option<u64> {
        Some(client_sequence)
    }
}

fn apply_input_frame_with_effective_policy() {
    if let Some(reason) = input_policy.reject_reason(frame.kind().as_policy_key()) {
        return InputApplyOutcome::rejected(reason);
    }
    if let Some(reason) = pointer_target_revision_reject_reason(frame, input_policy.pointer_target()) {
        return InputApplyOutcome::rejected(reason);
    }
    CGEventSetLocation(event, mapped_point(frame, target));
    let _guard_evidence = "target_guard_validation";
}

fn pointer_target_revision_reject_reason() -> Option<&'static str> {
    Some("stale_pointer_target_geometry")
}

fn record_rejection() {
    InputRejectSample::new(
        outcome.reason.unwrap_or("input_injection_failed"),
        rejected_count,
    )
    .client_sent_at_ms(frame.client_sent_at_ms());
    .client_sequence(frame.client_sequence());
}

fn input_frame_applied_payload() {
    let _ = client_sent_at_ms;
    let _ = client_sequence;
}

struct InputRejectSignature;
struct PendingInputReject;

struct InputRejectCoalescer {
    pending: BTreeMap<InputRejectSignature, PendingInputReject>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn pointer_policy_consumes_latest_target_tracker_snapshot() {
        let _ = policy["pointer_target"]["target_geometry_revision"];
    }

    #[test]
    fn current_session_input_policy_reapplies_session_input_scope_to_latest_snapshot() {
        assert!(
            !input_policy_allows(&policy, "pointer"),
            "current session policy must not let a loose base policy reopen view-only pointer input"
        );
        assert!(
            !input_policy_allows(&policy, "key"),
            "current session policy must not let a loose base policy reopen view-only keyboard input"
        );
    }

    #[test]
    fn input_policy_builder_canonicalizes_non_object_base_policy() {
        assert_eq!(policy["input_scope"], json!("view_only"));
        assert_eq!(policy["keyboard_enabled"], json!(false));
        assert_eq!(policy["pointer_enabled"], json!(false));
        assert_eq!(
            policy["unsupported_input_types"],
            json!(["clipboard", "file_drop"])
        );
        assert_eq!(
            input_policy_reject_reason(&policy, "pointer"),
            Some("input_scope_unsupported")
        );
    }

    #[test]
    fn current_session_input_policy_uses_same_geometry_revision_as_target_event() {}

    #[test]
    fn pointer_input_rejects_stale_target_geometry_revision_before_os_injection() {}

    #[test]
    fn target_local_input_without_bound_host_proof_fails_closed_before_os_injection() {}

    #[test]
    fn current_target_policy_replaces_creation_binding_after_rebind() {}

    #[test]
    fn target_pointer_mapping_clamps_raw_coordinates_inside_bound_surface() {}

    #[test]
    fn input_frame_applied_payload_preserves_client_timestamp() {}

    #[test]
    fn effective_input_policy_is_the_core_policy_object() {
        assert_eq!(outcome.reason, Some("input_policy_denied"));
        assert_eq!(view_only_pointer.reason, Some("input_scope_unsupported"));
        assert_eq!(view_only_key.reason, Some("input_scope_unsupported"));
        assert_eq!(clipboard_outcome.reason, Some("clipboard_input_unsupported"));
    }

    #[test]
    fn parse_input_frame_rejects_clipboard_and_file_drop_before_policy_application() {}

    #[test]
    fn maps_window_relative_pointer_to_global_screen_point() {
        assert!(!input_policy_allows(&policy, "pointer"));
    }

    #[test]
    fn maps_application_pointer_through_committed_union_surface_bounds() {
        assert!(!input_policy_allows(&policy, "pointer"));
    }

    #[test]
    fn input_reject_diagnostics_are_coalesced_across_interleaved_signatures() {}

    #[test]
    fn input_sequence_gate_rejects_replayed_or_out_of_order_frames() {
        assert_eq!(
            sequence_gate.reject_reason(Some(1)),
            Some("stale_client_sequence")
        );
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/target.rs" <<'RS'
enum TargetResolutionError {
    TargetIdentityAmbiguous,
    TargetMetadataIncomplete,
    TargetIdentityChanged,
}

enum InputScopeReason {
    TargetScopedInputGuarded,
}

impl InputScopeReason {
    fn as_str(&self) -> &'static str {
        match self {
            Self::TargetScopedInputGuarded => "target_scoped_input_guarded",
        }
    }
}

const TARGET_IDENTITY_AMBIGUOUS: TargetResolutionError =
    TargetResolutionError::TargetIdentityAmbiguous;
const TARGET_METADATA_INCOMPLETE: TargetResolutionError =
    TargetResolutionError::TargetMetadataIncomplete;

fn target_identity_ambiguous() {}

fn production_scope_ready() -> bool {
    !self.scope_audit.scope_widened && !self.scope_audit.display_fallback_used
}

fn target_bound_event_payload() {
    json!({
        "consent_epoch": self.consent_epoch,
        "input_scope_reason": self.scope_audit.input_scope_reason.as_str(),
        "scope_widened": self.scope_audit.scope_widened,
        "display_fallback_used": self.scope_audit.display_fallback_used,
    });
}

struct InputScopeDecision;

enum TargetScopedInputIsolation {
    MacosAccessibilityCoreGraphics,
    WindowsXcapUser32,
    LinuxX11Unisolated,
    Unsupported,
}

impl TargetScopedInputIsolation {
    const CURRENT: Self = if cfg!(target_os = "macos") {
        Self::MacosAccessibilityCoreGraphics
    } else if cfg!(all(target_os = "windows", feature = "native-media")) {
        Self::WindowsXcapUser32
    } else if cfg!(all(target_os = "linux", feature = "native-media")) {
        Self::LinuxX11Unisolated
    } else {
        Self::Unsupported
    };
}

struct AppWindowSetProof;

fn linux_x11_window_and_application_input_remain_view_only_without_press_release_isolation() {}

const LINUX_RECOVERY_GUARD: &str =
    "fresh X11 window-generation lease; recreate the session from fresh inventory";

struct AppSurfaceLayoutProof;

impl AppWindowSetProof {
    fn window_set_epoch(&self) -> u64 {
        2
    }
}

fn application_surface_rebind_candidate() {}

fn input_scope_for_request(target_isolation: TargetScopedInputIsolation) -> InputScopeDecision {
    match kind {
        RemoteDesktopTargetKind::Display => {
            let reason = "input_consent_required";
            InputScope::ViewOnly
        }
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
            if !target_isolation.is_safe() {
                let reason = "target_scoped_keyboard_pointer_dispatch_unsafe";
                return InputScope::ViewOnly;
            }
            InputScope::TargetLocal
        }
    }
}

fn validate_window() {
    return Err("window targets require owner pid, app_identity, or bundle_id");
}

fn validate_application() {
    return Err("application targets require primary_pid, app_identity, or bundle_id");
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_requires_stable_owner_identity_not_app_name_only() {}

    #[test]
    fn application_requires_stable_identity_and_exact_window_set() {}

    #[test]
    fn application_interactive_downgrade_projects_input_scope_reason() {
        assert_eq!(
            binding.scope_audit_value()["input_scope_reason"],
            json!("target_scoped_keyboard_pointer_dispatch_unsafe")
        );
        assert_eq!(
            binding.target_bound_event_payload()["input_scope_reason"],
            json!("target_scoped_keyboard_pointer_dispatch_unsafe")
        );
        assert_eq!(
            binding.target_bound_event_payload()["consent_epoch"],
            json!(binding.consent_epoch())
        );
    }

    #[test]
    fn application_interactive_with_input_consent_projects_guarded_target_scope() {}

    #[test]
    fn supported_platform_guards_admit_window_and_application_target_local_input() {}

    #[test]
    fn unsupported_platform_guard_keeps_target_local_input_fail_closed() {}

    #[test]
    fn display_interactive_downgrades_until_input_consent_exists() {
        assert_eq!(
            binding.scope_audit_value()["input_scope_reason"],
            json!("input_consent_required")
        );
        assert_eq!(
            binding.target_bound_event_payload()["input_scope_reason"],
            json!("input_consent_required")
        );
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs" <<'RS'
fn handle() {
    let target_binding_verifier = plugin.target_binding_verifier();
    let workflow = RemoteDesktopSessionCreationWorkflow::start(&env, &args)?
        .consume_consent(&registry, &env)?
        .resolve_target_with_verifier(target_binding_verifier.as_ref())?;
    let init = workflow.into_session_init()?;
    let session = RemoteDesktopSession::new(init);
    if let Err(err) =
        RemoteDesktopPlugin::schedule_session_lease(&plugin, watchdog_session_id.clone(), lease_expires_at_ms)
    {
        remove_inserted_session(&plugin, &tracker_session_id);
        return Err(err);
    }
    if let Err(err) = RemoteDesktopPlugin::track_session_target(&plugin, tracker_session_id) {
        plugin.cancel_session_lease(&watchdog_session_id);
        remove_inserted_session(&plugin, &tracker_session_id);
        return Err(err);
    }
}

fn remove_inserted_session(plugin: &RemoteDesktopPlugin, session_id: &str) {
    plugin.session_store().with_sessions(|sessions| {
        sessions.remove(session_id);
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn create_session_rejects_weak_window_identity_before_session_insert() {
        assert!(err.to_string().contains("target_identity_ambiguous"));
        assert!(!sessions.contains_key("rd-weak-window"));
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_creation.rs" <<'RS'
enum RemoteDesktopSessionCreationState {
    ReadyToInsert,
}

const READY_TO_INSERT: RemoteDesktopSessionCreationState =
    RemoteDesktopSessionCreationState::ReadyToInsert;

fn ensure_state(expected: RemoteDesktopSessionCreationState) -> anyhow::Result<()> {
    Ok(())
}

fn into_session_init(self) -> anyhow::Result<RemoteDesktopSessionInit> {
    let consent = self.consent.ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_CREATE_SESSION}: ready-to-insert workflow is missing consent"
        )
    })?;
    let target_binding = self.target_binding.ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_CREATE_SESSION}: ready-to-insert workflow is missing target binding"
        )
    })?;
    Ok(RemoteDesktopSessionInit { consent, target_binding })
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/invoke_bidi.rs" <<'RS'
fn handle_bidi_input_frame_for_session() {
    admit_input_host_effective_policy(
        session_store,
        session_id,
        InputTransportGuard::DiagnosticPreview,
        input_policy,
    );
    handle_parsed_bidi_input_frame_with_policy(&effective_input_policy, &frame);
}

fn handle_bidi_input_frame() {
    apply_input_frame_with_effective_policy(input_policy, frame);
}

fn attach_input_frame_telemetry(frame: RemoteDesktopInputFrame) {
    let _ = frame.client_sent_at_ms();
    let _ = frame.client_sequence();
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostic_bidi_view_only_input_reports_scope_unsupported() {}

    #[test]
    fn diagnostic_bidi_input_rechecks_session_target_snapshot() {}

    #[test]
    fn diagnostic_bidi_input_respects_session_policy() {
        assert_eq!(response["client_sequence"], json!(9_u64));
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/media-host/src/macos_sck.rs" <<'RS'
fn target_invalidated(detail: impl Into<String>) -> BackendFailure {
    BackendFailure::new(FailureReason::TargetInvalidated, detail)
}

fn select_window_for_binding() {
    target_invalidated(format!("ScreenCaptureKit window id {expected} is ambiguous"));
}

fn select_application_for_binding() {
    target_invalidated("ScreenCaptureKit application identity is ambiguous");
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/request.rs" <<'RS'
use crate::daemon::plugins::remote_desktop::input::RemoteDesktopInputPolicy;

fn request_projection() {
    let _policy: RemoteDesktopInputPolicy;
}

#[test]
fn input_policy_reports_clipboard_and_file_drop_unsupported_even_when_requested() {}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/input/linux.rs" <<'RS'
fn execute(target_guard: TargetInputGuardProof, pointer: Option<(i16, i16)>) {
    let grab = X11ServerGrab::begin(&self.connection)?;
    self.validate_target(target_guard, pointer)?;
    inject(self)?;
    self.barrier()?;
    grab.release()?;
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/target_observer.rs" <<'RS'
const X11_ATOMICITY: &str = "x11_server_grab";

fn observe_bound_session_target_once() {
    let Some(inputs) = sessions.target_observation_inputs_for_session(session_id) else {
        return TargetObservationPollResult::stop_tracking();
    };
    sessions.expire_target_rebind_deadline_for_session();
    TargetObservationPollResult::rebind_deadline_expired(media_source_lost);
    commit_target_observation_for_session();
}

fn validate_target_input_observation() {}

fn validate_target_pointer_input_observation() {}

enum TargetInputGuardFailure {
    PointerOutsideTargetSurface,
    PointerOccluded,
}

fn validate_live_target_pointer_input() {}

fn unsupported_platform_target_observation(binding: &RemoteAppTargetBinding) -> Option<TargetObservation> {
    match binding.target_kind() {
        RemoteDesktopTargetKind::Display => None,
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
            Some(TargetObservation::Lost {
                reason: TargetResolutionError::UnsupportedCaptureScope,
            })
        }
    }
}

fn observe_window() {
    if !owner_matches(binding, window) {
        return lost();
    }
    if window.visibility_state != TargetVisibilityState::Visible {
        return Some(TargetObservation::VisibilityChanged {
            visibility_state: window.visibility_state,
        });
    }
    if snapshot.title() != window.title.as_deref() {
        return Some(TargetObservation::TitleChanged {});
    }
    if snapshot.focused() != Some(window.focused) {
        return Some(TargetObservation::FocusChanged {});
    }
}

fn observe_application() {
    let committed_window_set = binding.committed_app_window_set().unwrap();
    let selected_display_windows = windows;
    let selected_display_window_ids = ids;
    let current_window_set = AppWindowSetProof::new(selected_display_window_ids);
    let current_surface_layout = AppSurfaceLayoutProof::from_front_to_back_geometries(windows);
    if &current_window_set != committed_window_set {
        return Some(TargetObservation::ApplicationSurfaceChanged {});
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn observation_provider_commits_through_session_store_boundary() {}

    #[test]
    fn observer_stops_tracking_missing_or_terminal_sessions_without_polling_provider() {}

    #[test]
    fn stale_observation_cannot_commit_after_session_binding_reuse() {}

    #[test]
    fn lost_observation_returns_media_source_stop_effect_after_debounce() {}

    #[test]
    fn window_observation_prioritizes_visibility_loss_over_title_or_focus_changes() {}

    #[test]
    fn application_observer_reports_committed_window_set_drift_as_rebind() {}

    #[test]
    fn application_observation_rebinds_same_display_window_set_expansion() {}

    #[test]
    fn application_observation_rebinds_same_app_window_set_subset() {}

    #[test]
    fn application_observer_rebinds_media_when_only_z_order_changes() {}

    #[test]
    fn application_pointer_guard_rejects_black_gaps_and_occluding_windows() {}

    #[test]
    fn snapshot_observer_reappearance_requires_explicit_rebind_policy() {}

    #[test]
    fn no_observation_tick_expires_rebind_deadline_before_polling_provider() {}

    #[test]
    fn pending_media_rebind_deadline_expiry_stops_active_endpoint_by_epoch() {}

    #[test]
    fn unsupported_platform_observer_fails_app_window_targets_closed() {}

    #[test]
    fn process_scoped_application_observer_tracks_window_set_without_display_identity() {}
}

RS

  cat >"$SANDBOX/plugins/remote-desktop/native-host/src/lib.rs" <<'RS'
fn sample_xcap_target_observations() {
    let windows = xcap::Window::all();
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/lease_monitor.rs" <<'RS'
struct RemoteDesktopLeaseMonitor {
    worker: Mutex<LifecycleWorker<LeaseMonitorCommand>>,
}

enum LeaseMonitorCommand {
    Shutdown,
}

fn shutdown(worker: &mut LifecycleWorker<LeaseMonitorCommand>) {
    worker.shutdown(LeaseMonitorCommand::Shutdown);
}

fn spawn_lease_monitor_worker() -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("easynet-rd-lease-monitor".into())
        .spawn(move || run_lease_monitor(plugin, rx))
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/target_monitor.rs" <<'RS'
use std::collections::HashSet;

enum TargetMonitorCommand {
    Track { session_id: String },
    Cancel { session_id: String },
    Shutdown,
}

struct RemoteDesktopTargetMonitor {
    worker: Mutex<LifecycleWorker<TargetMonitorCommand>>,
}

fn shutdown(worker: &mut LifecycleWorker<TargetMonitorCommand>) {
    worker.shutdown(TargetMonitorCommand::Shutdown);
}

fn apply_command(command: TargetMonitorCommand, tracked: &mut HashSet<String>) -> bool {
    match command {
        TargetMonitorCommand::Track { session_id } => {
            if !session_id.is_empty() {
                tracked.insert(session_id);
            }
            true
        }
        TargetMonitorCommand::Cancel { session_id } => {
            tracked.remove(&session_id);
            true
        }
        TargetMonitorCommand::Shutdown => false,
    }
}

fn spawn_target_monitor_worker() -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("easynet-rd-target-monitor".into())
        .spawn(move || run_target_monitor(plugin, rx))
}

fn apply_supervisor_command() {}

fn apply_generation_command() {}

fn spawn_target_monitor_generation() {}

#[cfg(test)]
mod tests {
    #[test]
    fn target_monitor_command_state_machine_tracks_cancels_and_shuts_down() {}

    #[test]
    fn snapshot_deadline_fences_late_result_and_bounds_native_call_count() {}

    #[test]
    fn provider_hang_exhausts_budget_without_spawning_unbounded_native_calls() {
        // plugin shutdown must not join the blocked native provider call
    }

    #[test]
    fn input_deadline_shares_monitor_single_flight_and_fences_monitor_result() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/target_snapshot.rs" <<'RS'
struct TargetSnapshotDeadlineExecutor;

enum TargetSnapshotOwner {
    MonitorGeneration,
    InputRequest,
}

fn sample_for_generation() {
    let _ = receiver.recv_timeout(remaining);
    if completed.owner != owner {
        return;
    }
}

fn sample_for_input() {
    let _ = TargetSnapshotOwner::InputRequest;
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/lifecycle_worker.rs" <<'RS'
fn shutdown(join: JoinHandle<()>) {
    if join.thread().id() == thread::current().id() {
        drop(join);
        return;
    }
    let _ = join.join();
}

#[cfg(test)]
mod tests {
    #[test]
    fn shutdown_from_worker_detaches_instead_of_self_joining() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/runtime.rs" <<'RS'
struct RemoteDesktopRuntime {
    lease_monitor: RemoteDesktopLeaseMonitor,
    target_monitor: RemoteDesktopTargetMonitor,
    target_binding_verifier: Arc<dyn RemoteAppTargetBindingVerifier>,
}

fn new() {
    Arc::new(PlatformRemoteAppTargetBindingVerifier);
}

fn schedule_session_lease(
    plugin: &Arc<RemoteDesktopPlugin>,
    session_id: String,
    lease_expires_at_ms: u64,
) -> anyhow::Result<()> {
    plugin
        .lease_monitor
        .schedule(plugin, session_id, lease_expires_at_ms)
}

fn track_session_target(plugin: &Arc<RemoteDesktopPlugin>, session_id: String) -> anyhow::Result<()> {
    plugin.target_monitor.track(plugin, session_id)
}

fn cancel_session_target_tracking(&self, session_id: &str) {
    self.target_monitor.cancel(session_id);
}

fn rehydrate_recovery_snapshots() {
    plugin
        .transport_manager()
        .observe_prior_epoch(snapshot.transport_epoch_high_watermark());
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_lifecycle.rs" <<'RS'
fn cleanup_session(plugin: RemoteDesktopPlugin, session_id: &str) {
    plugin.cancel_session_target_tracking(session_id);
}
RS
}

run_ok() {
  CHECK_REMOTEAPP_LIFECYCLE_INPUT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null
}

run_fail() {
  local expected="$1"
  if CHECK_REMOTEAPP_LIFECYCLE_INPUT_ROOT="$SANDBOX" bash "$SCRIPT" >"$SANDBOX/out" 2>"$SANDBOX/err"; then
    printf 'expected failure containing %s\n' "$expected" >&2
    exit 1
  fi
  rg -q -- "$expected" "$SANDBOX/err" || {
    printf 'expected failure containing %s, got:\n' "$expected" >&2
    cat "$SANDBOX/err" >&2
    exit 1
  }
}

write_fixture
run_ok

write_fixture
perl -0pi -e 's/Cross-display application window-set rebind is implemented/Cross-display application rebind remains incomplete/' \
  "$SANDBOX/docs/design/remoteapp-targeted-session-spec.md"
run_fail 'SPEC status must acknowledge the multi-surface application rebind path'

write_fixture
perl -0pi -e 's/Direct WebRTC route discovery is provider-backed/Direct WebRTC route discovery is host-only/' \
  "$SANDBOX/docs/design/remoteapp-targeted-session-spec.md"
run_fail 'SPEC status must acknowledge configured provider-backed STUN/TURN/EasyNet relay route discovery'

write_fixture
perl -0pi -e 's/struct RemoteDesktopEventProjection/type RemoteDesktopEventProjection = (&'\''static str, Value);\nstruct RetiredRemoteDesktopEventProjection/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'session event projection must be a domain object, not a tuple alias'

write_fixture
perl -0pi -e 's/direct_webrtc_endpoint_ura\(session\.session_id\(\)\)/legacy_endpoint(session.session_id())/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'public transport view must derive endpoint_ura from the canonical direct WebRTC endpoint helper'

write_fixture
perl -0pi -e 's#"relay_unavailable";#"relay_unavailable"; "webrtc://direct/legacy";#' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'remote desktop endpoint_ura evidence must be EasyNet URA only'

write_fixture
perl -0pi -e 's/ABILITY_ATTACH_SESSION/"screen.subscribe"/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'remote desktop diagnostic preview must not project the unrelated screen.subscribe ability'

write_fixture
perl -0pi -e 's/ABILITY_ATTACH_SESSION/"screen.subscribe"/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'remote desktop diagnostic preview must not project the unrelated screen.subscribe ability'

write_fixture
perl -0pi -e 's/preview_ability": ABILITY_ATTACH_SESSION/preview_ability": "remote_desktop.attach"/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport summary must project remote_desktop.attach as the diagnostic preview ability'

write_fixture
perl -0pi -e 's/preview_ability": ABILITY_ATTACH_SESSION/preview_ability": "remote_desktop.attach"/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'session-created event must project remote_desktop.attach as the diagnostic preview ability'

write_fixture
perl -0pi -e 's/transport_summary_projects_remote_desktop_attach_as_preview_ability/transport_summary_projects_screen_subscribe_as_preview_ability/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport tests must prove preview_ability uses the remote desktop attach ability'

write_fixture
perl -0pi -e 's/session_created_projects_remote_desktop_attach_as_preview_ability/session_created_projects_screen_subscribe_as_preview_ability/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'event tests must prove preview_ability uses the remote desktop attach ability'

write_fixture
perl -0pi -e 's/tracker_commits_move_resize_and_lost_without_rebinding/tracker_misses_regression/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'E2E-08 must have move/resize/lost tracker regression coverage'

write_fixture
perl -0pi -e 's/target_loss_pending/target_loss_unblocked/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'pending target loss debounce must block input before committed target loss'

write_fixture
perl -0pi -e 's/fn input_blocked_reason\(\)/fn input_blocked_reason_regression()/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'target snapshot must derive a single machine-readable input block reason'

write_fixture
perl -0pi -e 's/tracker_debounces_single_transient_lost_observation/tracker_debounce_does_not_block_input/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'target tracker must test pending-loss debounce input safety'

write_fixture
perl -0pi -e 's/TargetObservation::PermissionVerificationRequired/TargetObservation::PermissionRevoked/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate must consume fail-closed permission verification observations before confirmed revocation'

write_fixture
perl -0pi -e 's/PermissionVerificationPending/PermissionDenied/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'target state machine must persist an explicit recoverable permission verification phase'

write_fixture
perl -0pi -e 's/TARGET_PERMISSION_VERIFICATION_PENDING/TARGET_PERMISSION_REVOKED_PENDING/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'first negative permission sample must emit a typed pending-verification event'

write_fixture
perl -0pi -e 's/TARGET_PERMISSION_VERIFICATION_CLEARED/TARGET_PERMISSION_RESTORED_UNTYPED/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'positive permission evidence must emit a typed verification-cleared event'

write_fixture
perl -0pi -e 's/first_permission_denial_suspends_media_without_revoking_consent_or_session/first_permission_denial_revokes_consent/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session tests must prove the first negative permission sample is fail-closed without revoking consent or closing the session'

write_fixture
perl -0pi -e 's/permission_verification_is_fail_closed_recoverable_and_durable/permission_verification_is_terminal/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'target tests must prove pending permission verification survives recovery and can clear'

write_fixture
perl -0pi -e 's/pending_target_loss_deactivates_input_before_media_loss_debounce/pending_target_loss_keeps_input_active/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate must deactivate input during target-loss debounce before media loss commits'

write_fixture
perl -0pi -e 's/payload\["transport_epoch"\] = self\.transport\.active_epoch\(\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'target lifecycle event payloads must include current transport_epoch before event-log projection'

write_fixture
perl -0pi -e 's/self\.push_target_tracking_event\(event\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'record_target_observation must write target events through the session aggregate projection boundary'

write_fixture
perl -0pi -e 's/target_tracking_events_include_active_transport_epoch_at_session_boundary/target_tracking_events_drop_transport_epoch/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'E2E-08 must prove target lifecycle events carry the active transport epoch'

write_fixture
perl -0pi -e 's/target_lost_index < media_source_lost_index/media_source_lost_index < target_lost_index/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'E2E-09 must prove TARGET_LOST is ordered before MEDIA_SOURCE_LOST'

write_fixture
perl -0pi -e 's/REASON_TARGET_PERMISSION_REVOKED/REASON_PERMISSION_STILL_SUSPENDED/g' \
  "$SANDBOX/plugins/remote-desktop/src/constants.rs" \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'permission revocation must use a stable terminal reason code'

write_fixture
perl -0pi -e 's/fn begin_close_after_permission_revoked/fn suspend_after_permission_revoked/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'permission revocation must close through a dedicated aggregate terminal path'

write_fixture
perl -0pi -e 's/terminal_receipt/revoked_terminal_receipt/g' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'the common settled terminal path must publish a reason-bound RemoteApp terminal receipt'

write_fixture
perl -0pi -e 's/consent_revocation_terminates_session_and_blocks_input_activation/consent_revocation_suspends_media_and_blocks_input_activation/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'consent revocation must have session-level terminal media/input regression coverage'

write_fixture
perl -0pi -e 's/target_failure_payload/target_failure_projection_regression/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'target failure events must share one projection for failure-domain recovery fields'

write_fixture
perl -0pi -e 's/assert_eq!\(events\[target_lost_index\]\["payload"\]\["frontend_action"\], json!\("refresh_targets"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'E2E-09 must assert TARGET_LOST carries frontend recovery action'

write_fixture
perl -0pi -e 's/display_topology_loss_projects_target_failure_recovery/display_topology_loss_missing_recovery/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'display topology loss must have target-domain failure recovery coverage'

write_fixture
perl -0pi -e 's/json!\("target_display_unavailable"\)/json!("display_topology_changed")/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'display topology loss test must assert target_display_unavailable reason code'

write_fixture
perl -0pi -e 's/assert_eq!\(topology_changed\.payload\(\)\["input_blocked_reason"\], json!\("target_display_unavailable"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'display topology loss test must assert input_blocked_reason for frontend recovery'

write_fixture
perl -0pi -e 's/TargetResolutionError::TargetHidden/TargetVisibilityState::Hidden/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'hidden target visibility must use canonical target_hidden reason'

write_fixture
perl -0pi -e 's/json!\("target_hidden"\)/json!("target_hidden_regression")/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'hidden visibility test must assert target_hidden reason code'

write_fixture
perl -0pi -e 's/json!\("target_minimized"\)/json!("target_minimized_regression")/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'minimized visibility test must assert target_minimized reason code'

write_fixture
perl -0pi -e 's/assert_eq!\(hidden\.payload\(\)\["input_blocked_reason"\], json!\("target_hidden"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'hidden visibility test must assert input_blocked_reason for frontend recovery'

write_fixture
perl -0pi -e 's/assert_eq!\(minimized\.payload\(\)\["input_blocked_reason"\], json!\("target_minimized"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'minimized visibility test must assert input_blocked_reason for frontend recovery'

write_fixture
perl -0pi -e 's/json!\("retry_session"\)/json!("refresh_targets")/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'hidden/minimized visibility tests must assert canonical retry_session action'

write_fixture
perl -0pi -e 's/FrontendAction::RetrySession/FrontendAction::RefreshTargets/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'focus-loss target visibility must use canonical retry_session action'

write_fixture
perl -0pi -e 's/json!\("target_blurred"\)/json!("target_focused")/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'focus loss test must assert target_blurred reason code'

write_fixture
perl -0pi -e 's/assert_eq!\(blurred\.payload\(\)\["input_blocked_reason"\], json!\("target_blurred"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'focus loss test must assert input_blocked_reason for frontend recovery'

write_fixture
perl -0pi -e 's/if window\.visibility_state != TargetVisibilityState::Visible \{\n        return Some\(TargetObservation::VisibilityChanged \{\n            visibility_state: window\.visibility_state,\n        \}\);\n    \}\n    if snapshot\.title\(\) != window\.title\.as_deref\(\)/if snapshot.title() != window.title.as_deref()/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'window observer must prioritize hidden/minimized availability before title/focus updates'

write_fixture
perl -0pi -e 's/window_observation_prioritizes_visibility_loss_over_title_or_focus_changes/window_observation_allows_title_to_mask_hidden_state/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'target observer tests must prove hidden/minimized availability outranks title/focus updates'

write_fixture
perl -0pi -e 's/sample_xcap_target_observations/sample_stub_target_observations/g' \
  "$SANDBOX/plugins/remote-desktop/native-host/src/lib.rs"
run_fail 'native target observation must execute in the plugin-private native host'

write_fixture
perl -0pi -e 's/xcap::Window::all\(\)/Vec::new()/' \
  "$SANDBOX/plugins/remote-desktop/native-host/src/lib.rs"
run_fail 'native host must sample live xcap windows on compiled desktop platforms'

write_fixture
perl -0pi -e 's/process_scoped_application_observer_tracks_window_set_without_display_identity/process_scoped_application_observer_ignores_window_set/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'target observer tests must prove Windows/Linux process-scoped application window-set tracking'

write_fixture
perl -0pi -e 's/workflow\.into_session_init\(\)\?/workflow.into_session_init()/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"
run_fail 'create_session must construct the session only after fallible ready-to-insert conversion'

write_fixture
perl -0pi -e 's/(fn ensure_state\([^}]+\}\n)/$1\nfn assert_state() {}\n/s' \
  "$SANDBOX/plugins/remote-desktop/src/session_creation.rs"
run_fail 'creation workflow state transitions must not use panic assertions'

write_fixture
perl -0pi -e 's/(fn into_session_init\(self\) -> anyhow::Result<RemoteDesktopSessionInit> \{\n)/$1    self.consent.expect("ReadyToInsert workflow must contain consent");\n/s' \
  "$SANDBOX/plugins/remote-desktop/src/session_creation.rs"
run_fail 'ready-to-insert conversion must not prove consent/target binding with expect'

write_fixture
perl -0pi -e 's/if let Err\(err\) =\n        RemoteDesktopPlugin::schedule_session_lease\(&plugin, watchdog_session_id\.clone\(\), lease_expires_at_ms\)\n    \{\n        remove_inserted_session\(&plugin, &tracker_session_id\);\n        return Err\(err\);\n    \}//' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"
run_fail 'create_session must register created sessions with the lease monitor before returning'

write_fixture
perl -0pi -e 's/if let Err\(err\) = RemoteDesktopPlugin::track_session_target\(&plugin, tracker_session_id\) \{\n        plugin\.cancel_session_lease\(&watchdog_session_id\);\n        remove_inserted_session\(&plugin, &tracker_session_id\);\n        return Err\(err\);\n    \}//' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"
run_fail 'create_session must register created sessions with the target monitor'

write_fixture
perl -0pi -e 's/remove_inserted_session\(&plugin, &tracker_session_id\);//g' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"
run_fail 'create_session must roll back the inserted row when monitor registration fails'

write_fixture
perl -0pi -e 's/plugin\.cancel_session_lease\(&watchdog_session_id\);//' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"
run_fail 'create_session must cancel the lease monitor if target tracking registration fails'

write_fixture
perl -0pi -e 's/(\.spawn\(move \|\| run_lease_monitor\(plugin, rx\)\))/$1\n        .expect("spawn remote desktop lease monitor")/' \
  "$SANDBOX/plugins/remote-desktop/src/lease_monitor.rs"
run_fail 'lease monitor worker spawn must propagate errors instead of panicking'

write_fixture
perl -0pi -e 's/(\.spawn\(move \|\| run_target_monitor\(plugin, rx\)\))/$1\n        .expect("spawn remote desktop target monitor")/' \
  "$SANDBOX/plugins/remote-desktop/src/target_monitor.rs"
run_fail 'target monitor worker spawn must propagate errors instead of panicking'

write_fixture
perl -0pi -e 's/join\.thread\(\)\.id\(\) == thread::current\(\)\.id\(\)/false/' \
  "$SANDBOX/plugins/remote-desktop/src/lifecycle_worker.rs"
run_fail 'lifecycle worker must not join itself when the final owner drops on the worker thread'

write_fixture
perl -0pi -e 's/shutdown_from_worker_detaches_instead_of_self_joining/shutdown_from_worker_joins_itself/' \
  "$SANDBOX/plugins/remote-desktop/src/lifecycle_worker.rs"
run_fail 'lifecycle worker must test worker-thread destruction without self-join panic'

write_fixture
perl -0pi -e 's/plugin\.cancel_session_target_tracking\(session_id\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session_lifecycle.rs"
run_fail 'terminal session cleanup must cancel target tracking'

write_fixture
perl -0pi -e 's/return TargetObservationPollResult::stop_tracking\(\);/return TargetObservationPollResult::keep_tracking();/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'target observer must return stop_tracking when the session is missing or terminal'

write_fixture
perl -0pi -e 's/observer_stops_tracking_missing_or_terminal_sessions_without_polling_provider/observer_keeps_tracking_terminal_sessions/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'target observer tests must prove missing/terminal sessions stop tracking without polling host state'

write_fixture
perl -0pi -e 's/TargetMonitorCommand::Cancel \{ session_id \} => \{\n            tracked\.remove\(&session_id\);\n            true\n        \}/TargetMonitorCommand::Cancel { session_id } => true/' \
  "$SANDBOX/plugins/remote-desktop/src/target_monitor.rs"
run_fail 'target monitor Cancel command must remove the session id from the tracked set'

write_fixture
perl -0pi -e 's/target_monitor_command_state_machine_tracks_cancels_and_shuts_down/target_monitor_command_state_machine_missing/' \
  "$SANDBOX/plugins/remote-desktop/src/target_monitor.rs"
run_fail 'target monitor must test track/cancel/shutdown command semantics'

write_fixture
perl -0pi -e 's/tracker_reports_rebind_failure_after_target_loss_without_policy/tracker_swallows_rebind_signal/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'target tracker must test explicit rebind failure instead of silently swallowing post-loss observations'

write_fixture
perl -0pi -e 's/assert_eq!\(rebind_attempted\.payload\(\)\["failure_domain"\], json!\("target"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'TARGET_REBIND_ATTEMPTED must project target failure domain'

write_fixture
perl -0pi -e 's/assert_eq!\(rebind_failed\.payload\(\)\["failure_domain"\], json!\("target"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'TARGET_REBIND_FAILED must project target failure domain'

write_fixture
perl -0pi -e 's/"target_focus_after_loss";//' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'post-loss focus observations must enter explicit rebind instead of being silently swallowed'

write_fixture
perl -0pi -e 's/tracker_routes_post_loss_title_focus_through_explicit_rebind/tracker_swallows_title_focus_reappearance/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'target tracker must test title/focus reappearance through explicit rebind semantics'

write_fixture
perl -0pi -e 's/commit_pending_media_rebind_failed/commit_pending_media_rebuild_error_removed/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'target tracking must terminate failed pending media rebinds as typed target lifecycle events'

write_fixture
perl -0pi -e 's/active_application_window_set_rebind_failure_is_typed/active_application_window_set_rebind_failure_is_untyped/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'target tracker must test pending media rebind failure as TARGET_REBIND_FAILED'

write_fixture
perl -0pi -e 's/fn window_set_epoch/fn window_set_epoch_removed/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"
run_fail 'application window-set proof must expose the recomputed identity epoch'

write_fixture
perl -0pi -e 's/ApplicationSurfaceChanged/ApplicationSurfaceUnchecked/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'application observer must report window-set, geometry, and z-order drift as one media rebind observation'

write_fixture
perl -0pi -e 's/AppWindowSetProof::new/AppWindowSetProof::unchecked/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'application observer must rederive the current global app window-set proof'

write_fixture
perl -0pi -e 's/AppSurfaceLayoutProof::from_front_to_back_geometries/AppSurfaceLayoutProof::unchecked/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'application observer must rederive ordered surface geometry instead of treating identity as layout'

write_fixture
perl -0pi -e 's/application_observer_reports_committed_window_set_drift_as_rebind/application_observer_allows_window_set_drift/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'target observer must test committed application window-set expansion/contraction rebind evidence'

write_fixture
perl -0pi -e 's/application_observation_rebinds_same_display_window_set_expansion/application_observation_allows_same_display_window_set_expansion/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'application observer must test same-display app window-set expansion rebind evidence'

write_fixture
perl -0pi -e 's/application_observation_rebinds_same_app_window_set_subset/application_observation_allows_observer_subset_of_committed_capture_set/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'application observer must test same-display app window-set contraction rebind evidence'

write_fixture
perl -0pi -e 's/application_observer_rebinds_media_when_only_z_order_changes/application_observer_ignores_z_order_changes/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'application observer must rebuild application composition when only z-order changes'

write_fixture
perl -0pi -e 's/snapshot_observer_reappearance_requires_explicit_rebind_policy/snapshot_observer_reappearance_revives_stale_media/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'target observer must prove platform-visible target reappearance cannot revive media/input without explicit rebind policy'

write_fixture
perl -0pi -e 's/assert_eq!\(rebind_attempted\["reason_code"\], json!\("target_rebind_attempted"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate must assert TARGET_REBIND_ATTEMPTED top-level reason_code'

write_fixture
perl -0pi -e 's/assert_eq!\(rebind_attempted\["recoverability"\], json!\("retry_session"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate must assert TARGET_REBIND_ATTEMPTED top-level recoverability'

write_fixture
perl -0pi -e 's/assert_eq!\(rebind_attempted\["binding_id"\], json!\(session\.target_binding\(\)\.binding_id\(\)\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate must assert TARGET_REBIND_ATTEMPTED top-level binding id'

write_fixture
perl -0pi -e 's/assert_eq!\(rebind_failed\["reason_code"\], json!\("explicit_rebind_required"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate must assert TARGET_REBIND_FAILED top-level reason_code'

write_fixture
perl -0pi -e 's/assert_eq!\(rebind_failed\["recoverability"\], json!\("new_session_required"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate must assert TARGET_REBIND_FAILED top-level recoverability'

write_fixture
perl -0pi -e 's/assert_eq!\(rebind_failed\["target_identity_epoch"\], json!\(session\.target_binding\(\)\.target_identity_epoch\(\)\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate must assert TARGET_REBIND_FAILED top-level target identity epoch'

write_fixture
perl -0pi -e 's/pending_media_rebind_candidate_failure_restores_active_session/pending_media_rebind_candidate_failure_degrades_active_session/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate must preserve the committed media generation when a pending candidate fails'

write_fixture
perl -0pi -e 's/supersede_pending_media_rebind_for_session/native_rebind_supersession_bridge_removed/' \
  "$SANDBOX/plugins/remote-desktop/src/session_store.rs"
run_fail 'session store must expose the aggregate-owned supersession path for rejected native rebind candidates'

write_fixture
perl -0pi -e 's/supersede_pending_media_rebind_for_session/accept_invalid_pending_media_rebind/g' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs"
run_fail 'hosted WebRTC must reject an invalid replacement generation through the session aggregate'

write_fixture
perl -0pi -e 's/&active_binding/&pending.binding/g' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs"
run_fail 'hosted WebRTC replacement failure must restore media from the still-committed active binding'

write_fixture
perl -0pi -e 's/TARGET_CHANGED_EVENT_TYPES\.contains\(&event_type\)/TARGET_CHANGED_EVENT_TYPES.is_empty()/' \
  "$SANDBOX/plugins/remote-desktop/src/event_log.rs"
run_fail 'event log proto projection must consume the centralized target lifecycle taxonomy'

write_fixture
perl -0pi -e 's/#\[test\]\nfn spec_target_lifecycle_events_have_explicit_proto_projection\(\) \{\}//' \
  "$SANDBOX/plugins/remote-desktop/src/event_log.rs"
run_fail 'event log tests must prove every SPEC target lifecycle event has an explicit proto projection'

write_fixture
perl -0pi -e 's/let consent_epoch = target_field\("consent_epoch"\);//' \
  "$SANDBOX/plugins/remote-desktop/src/event_log.rs"
run_fail 'event log must lift consent epoch from target event payloads'

write_fixture
perl -0pi -e 's/"consent_epoch": consent_epoch,//' \
  "$SANDBOX/plugins/remote-desktop/src/event_log.rs"
run_fail 'watch-events rows must project consent epoch as a top-level field'

write_fixture
perl -0pi -e 's/session_events::media_source_lost\(self\.target\.binding\(\)\)/session_events::media_source_lost()/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'MEDIA_SOURCE_LOST projection must consume the committed session target binding'

write_fixture
perl -0pi -e 's/assert_eq!\(payload\["consent_epoch"\], json!\(binding\.consent_epoch\(\)\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'session event tests must prove target binding events publish consent epoch'

write_fixture
perl -0pi -e 's/(fn media_source_lost\([\s\S]*?)\n        "consent_epoch": binding\.consent_epoch\(\),/$1/s' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'MEDIA_SOURCE_LOST payload must carry consent epoch'

write_fixture
perl -0pi -e 's/WebRtcFailureEventKind::MediaSourceLost/WebRtcFailureEventKind::TransportFailed/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_media.rs"
run_fail 'native target failures must be projected as media-source loss, not generic session failure'

write_fixture
perl -0pi -e 's/Self::TransportFailed => "TRANSPORT_FAILED"/Self::TransportFailed => "SESSION_FAILED"/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'WebRTC transport failures must project TRANSPORT_FAILED'

write_fixture
perl -0pi -e 's/"reason_code": TargetResolutionError::TransportRouteUnavailable\.as_str\(\),//' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'direct WebRTC transport failure context must publish canonical transport_route_unavailable reason_code'

write_fixture
perl -0pi -e 's/"recoverability": "retry_session",//' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'direct WebRTC transport failure context must publish retry_session recoverability'

write_fixture
perl -0pi -e 's/"failure_domain": "transport"/"failure_domain": "session"/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'direct WebRTC transport failure context must identify the transport domain'

write_fixture
perl -0pi -e 's/FrontendAction::RetrySession\.as_str\(\)/"close_session"/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'direct WebRTC transport failure context must publish retry_session recovery action'

write_fixture
perl -0pi -e 's/webrtc_transport_failure_context\(\);/Value::Null;/' \
  "$SANDBOX/plugins/remote-desktop/src/session_store.rs"
run_fail 'direct WebRTC default failure path must not emit empty transport failure context'

write_fixture
perl -0pi -e 's/direct_webrtc_transport_failure_suspends_session_for_a_new_generation/direct_webrtc_transport_failure_terminates_session/' \
  "$SANDBOX/plugins/remote-desktop/src/session_store.rs"
run_fail 'session-store tests must prove transport failure preserves the session for a newer epoch'

write_fixture
perl -0pi -e 's/assert_eq!\(event\["reason_code"\], json!\("transport_route_unavailable"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session_store.rs"
run_fail 'session-store tests must prove TRANSPORT_FAILED top-level reason_code is projected'

write_fixture
perl -0pi -e 's/assert_eq!\(event\["recoverability"\], json!\("retry_session"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session_store.rs"
run_fail 'session-store tests must prove TRANSPORT_FAILED top-level recoverability is projected'

write_fixture
perl -0pi -e 's/"reason_code": "session_created",//' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'SESSION_CREATED path must publish initial reason_code and continue recoverability'

write_fixture
perl -0pi -e 's/session_created_payload_projects_initial_reason_code/session_created_payload_lacks_initial_reason_code/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'session event tests must prove SESSION_CREATED publishes initial reason_code'

write_fixture
perl -0pi -e '$n=0; s/"binding_id": binding\.binding_id\(\)/++$n == 1 ? "\"binding_id_omitted\": binding.binding_id()" : $&/ge' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'CAPTURE_TARGET_RESOLVED path must publish initial binding context and continue recoverability'

write_fixture
perl -0pi -e 's/assert_eq!\(payload\["consent_epoch"\], json!\(binding\.consent_epoch\(\)\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'session event tests must prove target binding events publish consent epoch'

write_fixture
perl -0pi -e 's/initial_session_events_project_reason_codes_in_order/initial_session_events_lack_reason_code_projection/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate tests must prove initial event-log top-level reason_code order'

write_fixture
perl -0pi -e 's/assert!\(created_index < resolved_index && resolved_index < bound_index\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate tests must prove initial session, resolution, and bound events are ordered'

write_fixture
perl -0pi -e 's/assert_eq!\(resolved\["binding_id"\], json!\(session\.target_binding\(\)\.binding_id\(\)\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate tests must prove CAPTURE_TARGET_RESOLVED top-level binding id is projected'

write_fixture
perl -0pi -e 's/"recoverability": "closing",//' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'SESSION_CLOSING path must publish terminal reason_code and closing recoverability'

write_fixture
perl -0pi -e 's/session_closing_payload_projects_terminal_reason_code/session_closing_payload_lacks_terminal_reason_code/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'session event tests must prove closing payload publishes terminal reason_code'

write_fixture
perl -0pi -e 's/(fn session_closed\((?:(?!fn session_expired).)*?)"reason_code": reason,/$1/s' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'SESSION_CLOSED caller path must publish terminal reason_code and closed recoverability'

write_fixture
perl -0pi -e 's/"recoverability": "closed",//' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'SESSION_CLOSED caller path must publish terminal reason_code and closed recoverability'

write_fixture
perl -0pi -e 's/session_closed_payload_projects_terminal_reason_code/session_closed_payload_lacks_terminal_reason_code/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'session event tests must prove caller close payload publishes terminal reason_code'

write_fixture
perl -0pi -e 's/session_expired_payload_projects_terminal_reason_code/session_expired_payload_lacks_terminal_reason_code/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'session event tests must prove lease expiry payload publishes terminal reason_code'

write_fixture
perl -0pi -e 's/session_close_events_project_terminal_reason_code/session_close_events_lack_terminal_reason_code/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate tests must prove close event-log top-level reason_code is projected'

write_fixture
perl -0pi -e 's/assert!\(closing_index < closed_index\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate tests must prove SESSION_CLOSING precedes terminal SESSION_CLOSED'

write_fixture
perl -0pi -e 's/assert_eq!\(closing\["recoverability"\], json!\("closing"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate tests must prove SESSION_CLOSING top-level recoverability is projected'

write_fixture
perl -0pi -e 's/session_expiry_events_project_terminal_reason_code/session_expiry_events_lack_terminal_reason_code/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate tests must prove expiry event-log top-level reason_code is projected'

write_fixture
perl -0pi -e 's/(fn media_source_lost\((?:(?!fn transport_blocked).)*?)"binding_id": binding\.binding_id\(\),/$1/s' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'MEDIA_SOURCE_LOST payload must carry binding id from the committed binding'

write_fixture
perl -0pi -e 's/TargetResolutionError::CaptureBackendUnavailable/TargetResolutionError::TransportRouteUnavailable/' \
  "$SANDBOX/plugins/remote-desktop/src/transport_blocker.rs"
run_fail 'backend-unavailable WebRTC blockers must map to the SPEC capture_backend_unavailable reason code'

write_fixture
perl -0pi -e 's/RemoteDesktopTransportBlocker::from_webrtc_error/legacy_transport_reason_from_string/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport readiness reason_code must reuse the shared non-route blocker taxonomy helper'

write_fixture
perl -0pi -e 's/RemoteDesktopTransportBlocker::from_webrtc_error/legacy_transport_reason_from_string/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'TRANSPORT_BLOCKED events must reuse the shared non-route blocker taxonomy helper'

write_fixture
perl -0pi -e 's/"reason_code": blocker\.map\(RemoteDesktopTransportBlocker::reason_code_str\),//' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'TRANSPORT_BLOCKED events must project canonical blocker reason_code'

write_fixture
perl -0pi -e 's/transport_blocked_projects_capture_backend_reason_code/transport_blocked_lacks_reason_code/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'session event tests must prove TRANSPORT_BLOCKED publishes capture_backend_unavailable'

write_fixture
perl -0pi -e 's/(fn mark_backend_unavailable\(\) \{\n)/$1    session.set_description("remote", description)?;\n/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_negotiation.rs"
run_fail 'transport backend-unavailable gate must not partially commit remote SDP signaling'

write_fixture
perl -0pi -e 's/remote_offer_backend_gate_blocks_without_committing_signaling/remote_offer_backend_gate_only_checks_transport_block/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/set_description.rs"
run_fail 'set_description tests must prove backend-unavailable gate leaves signaling uncommitted'

write_fixture
perl -0pi -e 's/assert_eq!\(signaled\["signaling"\]\["remote_description"\], Value::Null\);//' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/set_description.rs"
run_fail 'backend-unavailable regression must assert remote_description remains empty'

write_fixture
perl -0pi -e 's/\.all\(\|event\| event\["event_type"\] != json!\("DESCRIPTION_SET"\)\)/.any(|event| event["event_type"] == json!("TRANSPORT_BLOCKED"))/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/set_description.rs"
run_fail 'backend-unavailable regression must assert DESCRIPTION_SET is not emitted'

write_fixture
perl -0pi -e 's/fn transport_route_state/fn transport_summary_without_routes/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport view must expose host/STUN/TURN/EasyNet relay route state'

write_fixture
perl -0pi -e 's/TargetResolutionError::TransportRouteUnavailable/TargetResolutionError::CaptureBackendUnavailable/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport route degradation must expose the SPEC canonical transport_route_unavailable reason code from the shared taxonomy'

write_fixture
perl -0pi -e 's/"message": self\.message,\s*"reason_code": self\.reason_code\.clone\(\),/"message": self.message,/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'public transport summary must project canonical route degradation reason_code'

write_fixture
perl -0pi -e 's/"input_channel_label": INPUT_DATA_CHANNEL_LABEL,\s*"reason_code": self\.reason_code\.clone\(\),/"input_channel_label": INPUT_DATA_CHANNEL_LABEL,/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'primary transport metadata must preserve canonical route degradation reason_code'

write_fixture
perl -0pi -e 's/fn transport_readiness_blocker/fn transport_detail_reason/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport route reason_code derivation must be centralized in the transport readiness blocker projection'

write_fixture
perl -0pi -e 's/fn transport_route_failed/fn transport_error_failed/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport route failed predicate must be explicit instead of treating every WebRTC error as a route failure'

write_fixture
perl -0pi -e 's/"host_only_no_nat_or_relay"/"webrtc_ice_connecting"/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'host-only transport degradation must have a typed unavailable reason'

write_fixture
perl -0pi -e 's/"relay_unavailable"/"webrtc_ice_connecting"/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'relay-unavailable transport degradation must have a typed unavailable reason'

write_fixture
perl -0pi -e 's/fn host_only_route_keeps_production_offline_after_client_media_presents/fn host_only_route_allows_production_online/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport tests must prove host-only routes cannot report production online after client media presents'

write_fixture
perl -0pi -e 's/fn candidate_declares_easynet_relay/fn candidate_mentions_easynet/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'EasyNet relay classification must require explicit relay metadata'

write_fixture
perl -0pi -e 's/"relay_unavailable";/"relay_unavailable"; candidate_text.contains("easynet");/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'EasyNet relay classification must not infer relay type from candidate hostname text'

write_fixture
perl -0pi -e 's/fn turn_relay_hostname_containing_easynet_is_not_easynet_relay/fn turn_relay_hostname_with_easynet_is_misclassified/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport tests must prove TURN hostnames containing easynet are not EasyNet relay routes'

write_fixture
perl -0pi -e 's/trait DirectWebRtcRouteCandidateProvider/trait DirectWebRtcAddressProvider/' \
  "$SANDBOX/plugins/remote-desktop/src/network.rs"
run_fail 'direct WebRTC endpoint route discovery must be provider-backed instead of a bare UDP address helper'

write_fixture
perl -0pi -e 's/struct DirectWebRtcRouteCandidate;/struct DirectWebRtcUdpAddress;/' \
  "$SANDBOX/plugins/remote-desktop/src/network.rs"
run_fail 'direct WebRTC route candidates must carry typed route-class evidence'

write_fixture
perl -0pi -e 's/TurnRelay/RelayOverloaded/g' \
  "$SANDBOX/plugins/remote-desktop/src/network.rs"
run_fail 'direct WebRTC route candidate model must reserve TURN relay evidence explicitly'

write_fixture
perl -0pi -e 's/assert_eq!\(evidence\["provider_state"\], json!\("host_local_only"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/network.rs"
run_fail 'local route candidate provider tests must prove host-only state is explicit'

write_fixture
perl -0pi -e 's/struct DirectWebRtcIceServerConfig;/struct DirectWebRtcRouteEndpoint;/g' \
  "$SANDBOX/plugins/remote-desktop/src/network.rs"
run_fail 'direct WebRTC route configuration must project typed ICE server config instead of raw endpoint strings'

write_fixture
perl -0pi -e 's/struct DirectWebRtcRouteConfig;/struct DirectWebRtcRouteBag;/g' \
  "$SANDBOX/plugins/remote-desktop/src/network.rs"
run_fail 'direct WebRTC route configuration must be a typed provider input'

write_fixture
perl -0pi -e 's/ConfiguredDirectWebRtcRouteProvider/StaticDirectWebRtcRouteProvider/g' \
  "$SANDBOX/plugins/remote-desktop/src/network.rs"
run_fail 'direct WebRTC route discovery must support configured STUN/TURN/EasyNet relay routes through the provider'

write_fixture
perl -0pi -e 's/credential_configured/credential/g' \
  "$SANDBOX/plugins/remote-desktop/src/network.rs"
run_fail 'direct WebRTC route evidence must redact credentials while proving relay credentials are configured'

write_fixture
perl -0pi -e 's/fn configured_ice_routes_do_not_become_local_udp_bind_endpoints/fn configured_routes_share_udp_bind_path/' \
  "$SANDBOX/plugins/remote-desktop/src/network.rs"
run_fail 'route tests must prove STUN/TURN/EasyNet relay URLs are never used as UDP bind addresses'

write_fixture
perl -0pi -e 's/fn configured_route_provider_projects_ice_servers_without_credentials_in_evidence/fn configured_route_provider_projects_ice_servers/' \
  "$SANDBOX/plugins/remote-desktop/src/network.rs"
run_fail 'route tests must prove configured ICE routes are represented without credential leakage'

write_fixture
perl -0pi -e 's/^/fn direct_webrtc_udp_addrs() {}\n/' \
  "$SANDBOX/plugins/remote-desktop/src/network.rs"
run_fail 'direct WebRTC route discovery must not regress to an untyped UDP address helper'

write_fixture
perl -0pi -e 's/"route_candidate_evidence": route_candidate_evidence,//' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
run_fail 'direct WebRTC answer must publish route candidate evidence for frontend/backend diagnosis'

write_fixture
perl -0pi -e 's/ConfiguredDirectWebRtcRouteProvider::from_env_with_relay_lease\(relay_lease\)/LocalInterfaceRouteCandidateProvider/g' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
run_fail 'direct WebRTC endpoint must consume the configured typed route provider'

write_fixture
perl -0pi -e 's/with_ice_servers/without_ice_servers/g' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
run_fail 'direct WebRTC endpoint must wire provider-backed ICE servers into RTC configuration'

write_fixture
perl -0pi -e 's/RTCIceServer/RawIceServer/g' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
run_fail 'direct WebRTC endpoint must translate typed route config into WebRTC ICE server configuration'

write_fixture
perl -0pi -e 's/candidate\.local_bind_endpoint\(\)/candidate.endpoint()/g' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
run_fail 'endpoint UDP bind addresses must be derived only from typed host bind candidates'

write_fixture
perl -0pi -e 's/filter_map/map/g' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
run_fail 'endpoint UDP bind addresses must filter out configured STUN/TURN/EasyNet relay URLs'

write_fixture
perl -0pi -e 's/let udp_addrs =/let _bad_route_endpoint = candidate.endpoint().to_string();\n    let udp_addrs =/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
run_fail 'endpoint UDP bind addresses must not treat every route candidate endpoint as a local bind address'

write_fixture
perl -0pi -e 's/"production_route_ready": transport_view\.production_route_ready\(\),//' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'public production readiness must expose production route readiness'

write_fixture
perl -0pi -e 's/assert_eq!\(summary\["reason_code"\], json!\("transport_route_unavailable"\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport tests must assert canonical route degradation reason_code'

write_fixture
perl -0pi -e 's/route_state/route_summary/g' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'public session view must project route state'

write_fixture
perl -0pi -e 's/\.suspend\(\)/.mark_degraded()/g' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'target loss must suspend the session lifecycle'

write_fixture
perl -0pi -e 's/\n    session_events::session_degraded\(state, epoch\.value\(\), "degraded"\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate must emit SESSION_DEGRADED from lifecycle health projection'

write_fixture
perl -0pi -e 's/client_media_stall_emits_session_degraded_recovery_event/client_media_stall_missing_lifecycle_projection/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate tests must prove client media stall emits SESSION_DEGRADED'

write_fixture
perl -0pi -e 's/assert!\(events\.iter\(\)\.all\(\|event\| event\["event_type"\] != json!\("SESSION_DEGRADED"\)\)\);//g' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'target-domain media source loss tests must assert SESSION_DEGRADED is not emitted'

write_fixture
perl -0pi -e 's/"failure_domain": "client_media"/"failure_domain": "target"/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'SESSION_DEGRADED must stay separate from target and transport failure domains'

write_fixture
perl -0pi -e 's/session_degraded_payload_projects_recovery_context/session_degraded_payload_missing_recovery_context/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'session event tests must prove SESSION_DEGRADED recovery payload shape'

write_fixture
perl -0pi -e 's/self\.target\.binding\(\)\.production_scope_ready\(\)\s*&&\s*//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'production media readiness must be gated by target binding scope readiness'

write_fixture
perl -0pi -e 's/\n        && self\.transport\.client_media_ready\(\)//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'production media readiness must wait for client presenting evidence, not only device sender readiness'

write_fixture
perl -0pi -e 's/"target_scope_ready": session\.target_scope_ready\(\),//' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'public production readiness must expose target scope readiness'

write_fixture
perl -0pi -e 's/let video_ready = transport_view\.production_ready\(session\);//' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'public production readiness must bind video readiness to media plus route readiness'

write_fixture
perl -0pi -e 's/let audio_required = negotiated_media_scope\.is_some_and\(\|scope\| scope\.requires_audio\(\)\);/let audio_required = true;/' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'public production readiness must require host audio only for audio-video negotiation'

write_fixture
perl -0pi -e 's/let ready = video_ready\s*&& negotiated_media_scope_ready\s*&& client_decode_ready\s*&& \(!audio_required \|\| audio_ready\);/let ready = video_ready;/s' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'public production readiness must require bound client decode, accept video-only sessions, and require audio evidence for audio-video sessions'

write_fixture
perl -0pi -e 's/video_only_negotiation_requires_bound_decode_but_not_audio_runtime_stats/video_only_negotiation_waits_for_unrequested_audio/' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'session view tests must prove video-only readiness requires bound decode without fabricating an audio requirement'

write_fixture
perl -0pi -e 's/audio_video_negotiation_requires_live_audio_runtime_stats/audio_video_negotiation_ignores_audio_runtime_stats/' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'session view tests must prove audio-video readiness requires live host-audio evidence'

write_fixture
perl -0pi -e 's/"ready": ready,//' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'public production readiness ready predicate must use the route-gated transport production predicate'

write_fixture
perl -0pi -e 's/"blocked_reason": production_readiness_blocked_reason\(/"blocked_reason": legacy_readiness_reason\(/' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'public production readiness must expose one typed blocked_reason instead of forcing UI inference'

write_fixture
perl -0pi -e 's/"client_media_not_presenting"/"production_readiness_incomplete"/' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'production readiness must distinguish missing client presenting/decoded evidence'

write_fixture
perl -0pi -e 's/"production_route_not_ready"/"production_readiness_incomplete"/' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'production readiness must distinguish route blockers after media and client presentation are ready'

write_fixture
perl -0pi -e 's/"production_ready": self\.production_ready\(session\),//' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport projection must expose route-gated production_ready separately from primary_ready'

write_fixture
perl -0pi -e 's/"display_fallback_used": self\.scope_audit\.display_fallback_used/"display_fallback_used": false/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"
run_fail 'TARGET_BOUND payload must project display fallback from the committed binding audit'

write_fixture
perl -0pi -e 's/"consent_epoch": self\.consent_epoch,//' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"
run_fail 'TARGET_BOUND payload must project consent epoch from the committed binding'

write_fixture
perl -0pi -e 's/assert_eq!\(target_bound\["consent_epoch"\], json!\(session\.target_binding\(\)\.consent_epoch\(\)\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session tests must assert TARGET_BOUND top-level consent epoch projection'

write_fixture
perl -0pi -e 's/"input_scope_reason": self\.scope_audit\.input_scope_reason\.as_str\(\),//g' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"
run_fail 'target binding and TARGET_BOUND projection must expose the committed input scope reason'

write_fixture
perl -0pi -e 's/"input_readiness": input_readiness\.clone\(\),//g' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'session view must expose a single machine-readable input readiness projection'

write_fixture
perl -0pi -e 's/view\["input_readiness"\]\["blocked_reason"\]/view["scope_audit"]["input_scope_reason"]/g' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'session view tests must assert input downgrade blocked reason in input_readiness'

write_fixture
perl -0pi -e 's/PrimaryMediaPhase::MediaSourceLost => [^\n]+/PrimaryMediaPhase::MediaSourceLost => false/' \
  "$SANDBOX/plugins/remote-desktop/src/session_transport_state.rs"
run_fail 'media source loss must be absorbing until failure or a new epoch'

write_fixture
perl -0pi -e 's/let snapshot = session\.target_snapshot\(\);/let snapshot = cached_snapshot;/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'production input path must read the latest session target snapshot'

write_fixture
perl -0pi -e 's/let binding = session\.target_binding\(\);/let binding = cached_binding;/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'production input path must read the current session-owned target binding'

write_fixture
perl -0pi -e 's/base_policy\.for_current_target\(snapshot, binding\);/base_policy;/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'production input path must reapply the current binding and snapshot through the typed effective policy'

write_fixture
perl -0pi -e 's/current_session_input_policy_uses_same_geometry_revision_as_target_event/current_session_input_policy_allows_stale_geometry_revision/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'E2E-08 must prove target event and input mapping consume the same committed geometry revision'

write_fixture
perl -0pi -e 's/target_geometry_revision: Option<u64>/target_geometry_revision_removed: Option<u64>/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'pointer input frames must carry the client-observed target geometry revision'

write_fixture
perl -0pi -e 's/fn pointer_target_revision_reject_reason/fn pointer_target_revision_accepts_stale_reason/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'input execution path must reject stale pointer frames before OS injection'

write_fixture
perl -0pi -e 's/stale_pointer_target_geometry/ignored_pointer_target_geometry/g' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'input execution path must expose a stable stale pointer geometry rejection reason'

write_fixture
perl -0pi -e 's/if let Some\(reason\) = input_policy\.reject_reason\(frame\.kind\(\)\.as_policy_key\(\)\) \{\n        return InputApplyOutcome::rejected\(reason\);\n    \}//' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'input frame application must enforce typed effective input policy before OS injection'

write_fixture
perl -0pi -e 's/effective_input_policy_is_the_core_policy_object/effective_input_policy_boundary_missing/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'input tests must prove the typed effective input policy is the core policy object'

write_fixture
perl -0pi -e 's/reject_unsupported_input_channel_frame\(frame\)\?;//' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'input frame validation must reject unsupported rich input before policy application'

write_fixture
perl -0pi -e 's/parse_input_frame_rejects_clipboard_and_file_drop_before_policy_application/parse_input_frame_accepts_rich_input/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'input parser tests must prove clipboard/file-drop fail before policy application'

write_fixture
perl -0pi -e 's/admit_input_host_effective_policy/static_input_policy/' \
  "$SANDBOX/plugins/remote-desktop/src/invoke_bidi.rs"
run_fail 'diagnostic bidi input path must atomically re-read readiness and admit each input frame'

write_fixture
perl -0pi -e 's/apply_input_frame_with_effective_policy\(input_policy, frame\)/input_policy_reject_reason(input_policy, kind)/' \
  "$SANDBOX/plugins/remote-desktop/src/invoke_bidi.rs"
run_fail 'diagnostic bidi input path must use the typed policy-enforced input application boundary'

write_fixture
perl -0pi -e 's/json!\(false\)/json!(true)/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'E2E-09 must assert target loss disables input'

write_fixture
perl -0pi -e 's/target_invalidated/target_failure_without_domain/g' \
  "$SANDBOX/plugins/remote-desktop/media-host/src/macos_sck.rs"
run_fail 'ScreenCaptureKit media-host must report native target ambiguity through the typed target-invalidated failure domain'

write_fixture
perl -0pi -e 's/target_identity_ambiguous/target_metadata_incomplete/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"
run_fail 'weak target identity handler test must assert target_identity_ambiguous'

write_fixture
perl -0pi -e 's/self\.pointer_enabled = false;/self.pointer_enabled = true;/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'view-only input policy must disable pointer'

write_fixture
perl -0pi -e 's/Some\("input_scope_unsupported"\)/Some("input_disabled")/g' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'view-only key/pointer rejection must report input_scope_unsupported'

write_fixture
perl -0pi -e 's/BTreeMap<InputRejectSignature, PendingInputReject>/Option<PendingInputReject>/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'input rejection coalescing must aggregate by signature instead of a single pending rejection'

write_fixture
perl -0pi -e 's/input_reject_diagnostics_are_coalesced_across_interleaved_signatures/input_reject_diagnostics_flush_on_signature_changes/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'PERF-07 must prove alternating invalid input signatures do not produce one diagnostic per frame'

write_fixture
perl -0pi -e 's/const TARGET_LIFECYCLE_EVENT_COALESCE_INTERVAL_MS: u64 = 100;//' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'move/resize/title event coalescing must be capped at 10Hz per session'

write_fixture
perl -0pi -e 's/struct TargetLifecycleEventCoalescer;/struct TargetLifecycleEventPerTypeCoalescer;/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'target lifecycle event coalescing must live in the session target state machine domain'

write_fixture
perl -0pi -e 's/tracker_coalesces_high_rate_geometry_and_title_events/tracker_coalesces_geometry_events_only/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'SPEC max 10Hz move/resize/title coalescing must have alternating-event regression coverage'

write_fixture
perl -0pi -e 's/"unsupported_input_types": unsupported_input_channel_types_value\(\),/"supported_input_types": unsupported_input_channel_types_value(),/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must report unsupported input channel types'

write_fixture
perl -0pi -e 's/"unsupported_capabilities":/"enabled_capabilities":/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must report unsupported rich-input capabilities'

write_fixture
perl -0pi -e 's/native_webrtc_backend_runtime_descriptor\(\)/MACOS_SCK_VIDEOTOOLBOX_BACKEND/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must derive production target subjects from the runtime native backend descriptor'

write_fixture
perl -0pi -e 's/let production_target_subjects = if production_ready \{\s*production_backend\.supported_subjects_value\(\)\s*\} else \{\s*json!\(\[\]\)\s*\};/let production_target_subjects = production_backend.supported_subjects_value();/s' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must gate production target subjects on runtime production readiness'

write_fixture
perl -0pi -e 's/production_backend\.supported_subjects_value\(\)/json!(["display"])/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must gate production target subjects on runtime production readiness'

write_fixture
perl -0pi -e 's/json!\(\[\]\)/json!(["display", "window", "application"])/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must gate production target subjects on runtime production readiness'

write_fixture
perl -0pi -e 's/"production_target_subjects": production_target_subjects,//' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose the native production backend display/window/application subject matrix'

write_fixture
perl -0pi -e 's/"diagnostic_target_subjects": diagnostic_target_subjects,//' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose diagnostic target subjects separately from production target subjects'

write_fixture
perl -0pi -e 's/XCAP_OPENH264_WEBRTC_BACKEND\.supported_subjects_value\(\)/json!(["display", "window"])/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must derive baseline target subjects from the xcap WebRTC backend'

write_fixture
perl -0pi -e 's/"production_target_subjects_blocked_reason": if production_ready \{\s*Value::Null\s*\} else \{\s*json!\(production_backend\.unavailable_reason\(\)\.unwrap_or\("production_backend_not_ready"\)\)\s*\},//s' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose why production app/window subjects are not claimable'

write_fixture
perl -0pi -e 's/"production_target_subjects_source": if production_ready \{\s*MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID\s*\} else \{\s*"none"\s*\},//s' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose the source backend for production target subjects'

write_fixture
perl -0pi -e 's/"platform_support": platform_support,//' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose product-visible platform support matrix'

write_fixture
perl -0pi -e 's/"input_control_support": input_control_support,//' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose product-visible input control support matrix'

write_fixture
perl -0pi -e 's/platform_support_view\(production_ready, &production_backend\)/platform_support_view(false, &production_backend)/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must derive platform support from runtime production readiness'

write_fixture
perl -0pi -e 's/input_control_support_view\(input_available, target_local_guard_available\)/input_control_support_view(true, false)/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must derive input control support from runtime input permission and compiled target guard'

write_fixture
perl -0pi -e 's/WindowsXcapUser32/WindowsUnavailable/g' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"
run_fail 'target input admission must make the Windows xcap/User32 guard reachable'

write_fixture
perl -0pi -e 's/supported_platform_guards_admit_window_and_application_target_local_input/supported_platform_guards_leave_target_input_unreachable/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"
run_fail 'target binding tests must prove macOS, Windows, and Linux compiled guards all admit target-local input'

write_fixture
perl -0pi -e 's/"target_local_guard_compiled": target_local_guard_available,//' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose compiled exact-target guard availability separately'

write_fixture
perl -0pi -e 's/input_capability_keeps_display_global_but_blocks_target_local_without_guard/input_capability_hides_missing_target_guard/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capability tests must prove a missing target guard cannot be hidden by an available global injector'

write_fixture
perl -0pi -e 's/linux_xcap_target_baseline_ready/linux_capture_unknown/g' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose Linux exact-target xcap baseline readiness'

write_fixture
perl -0pi -e 's/windows_xcap_target_baseline_ready/windows_capture_unknown/g' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose Windows exact-target xcap baseline readiness'

write_fixture
perl -0pi -e 's/"baseline_ready"/"production_ready"/g' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must distinguish executable baseline from certified production capture'

write_fixture
perl -0pi -e 's/"requires_input_control_consent": true/"requires_input_control_consent": false/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose explicit input-control consent requirement'

write_fixture
perl -0pi -e 's/macos_target_input_guard_ready/macos_input_unknown/g' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose guarded macOS window/application input readiness'

write_fixture
perl -0pi -e 's/linux_x11_xcb_atomic_display_global_ready/linux_input_unknown/g' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose Linux X11 display-global input readiness'

write_fixture
perl -0pi -e 's/fresh X11 window-generation lease; recreate the session from fresh inventory/Linux recovery accepted stale XID/g' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"
run_fail 'Linux Window/Application restart recovery must fail closed without an XID generation lease'

write_fixture
perl -0pi -e 's/windows_sendinput_target_guard_ready/windows_input_unknown/g' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose the guarded Windows SendInput baseline'

write_fixture
perl -0pi -e 's/"multi_surface_application_window_set"/"application"/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose the macOS multi-surface application target model'

write_fixture
perl -0pi -e 's/"process_scoped_application_window_set"/"application"/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose the Windows/Linux process-scoped application target model'

write_fixture
perl -0pi -e 's/"application_surface"/"application_scope"/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose application multi-window and multi-display constraints'

write_fixture
perl -0pi -e 's/"multi_surface",\s*true,\s*None/"multi_surface", false, None/s' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose macOS multi-surface multi-display application support'

write_fixture
perl -0pi -e 's/"process_scoped",\s*true,\s*None/"process_scoped", false, None/s' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose process-scoped Windows/Linux multi-display application support'

write_fixture
perl -0pi -e 's#display/window/application target capture#display capture#' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must describe native ScreenCaptureKit as targeted display/window/application capture'

write_fixture
perl -0pi -e 's/device_capabilities_project_native_target_subject_matrix/device_capabilities_hide_native_target_subject_matrix/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capability tests must prove the native target subject matrix is projected'

write_fixture
perl -0pi -e 's/device_capabilities_project_cross_platform_support_matrix/device_capabilities_hide_cross_platform_support_matrix/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capability tests must prove the cross-platform support matrix is projected'

write_fixture
perl -0pi -e 's/device_capabilities_project_input_control_support_matrix/device_capabilities_hide_input_control_support_matrix/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capability tests must prove the input control support matrix is projected'

write_fixture
perl -0pi -e 's/"unsupported_input_types": unsupported_input_channel_types_value\(\),/"unsupported_input_types": json!(["clipboard"]),/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'request input policy projection must reuse the input-domain unsupported type set'

write_fixture
perl -0pi -e 's/validate_target_pointer_input_observation/validate_pointer_without_host_surface_guard/g' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'target-local pointer execution must validate the mapped host point before OS injection'

write_fixture
perl -0pi -e 's/PointerOutsideTargetSurface/PointerOutsideUncheckedSurface/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'application canvas gaps must fail closed instead of targeting host desktop content'

write_fixture
perl -0pi -e 's/application_pointer_guard_rejects_black_gaps_and_occluding_windows/application_pointer_guard_allows_gaps_and_occlusion/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'pointer guard must regress both black-gap and foreign-window occlusion failures'

write_fixture
perl -0pi -e 's/fn commit_application_surface\(/fn commit_application_surface_unchecked(/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'target tracking must commit application identity and surface layout through one domain transition'

write_fixture
perl -0pi -e 's/fn mark_webrtc_generation_failed_with_context\(\) \{\s*self\.transport\.mark_failed\(epoch\);\s*self\.lifecycle\.suspend\(\);/fn mark_webrtc_generation_failed_with_context() { self.transport.mark_failed(epoch);/s' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'failed WebRTC generations must suspend the reusable product session'

write_fixture
perl -0pi -e 's/epoch\.value\(\) <= self\.epoch_high_watermark/epoch.value() < self.epoch_high_watermark/' \
  "$SANDBOX/plugins/remote-desktop/src/session_transport_state.rs"
run_fail 'transport state must reject reused or regressing epochs'

printf 'test_check_remoteapp_lifecycle_input_boundary.sh: all cases passed\n'
