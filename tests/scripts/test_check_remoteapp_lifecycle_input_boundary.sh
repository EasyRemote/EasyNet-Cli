#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-lifecycle-input-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p "$SANDBOX/docs/design"
mkdir -p "$SANDBOX/plugins/remote-desktop/src/handlers"
mkdir -p "$SANDBOX/plugins/remote-desktop/src/transport"

write_fixture() {
  rm -rf "$SANDBOX/docs" "$SANDBOX/plugins"
  mkdir -p "$SANDBOX/docs/design"
  mkdir -p "$SANDBOX/plugins/remote-desktop/src/handlers"
  mkdir -p "$SANDBOX/plugins/remote-desktop/src/transport"

  cat >"$SANDBOX/docs/design/remoteapp-targeted-session-spec.md" <<'MD'
| E2E-08 move/resize tracking | move and resize events advance target geometry revision and input consumes that revision |
| E2E-09 target loss vs transport failure | selected target loss emits target/media loss without transport failure |
| E2E-10 weak identity ambiguity | ambiguous weak native identity fails closed before stream start |
| E2E-11 view-only input safety | app/window sessions remain view-only without a focus-safe input validator |
relay_ready
MD

  cat >"$SANDBOX/plugins/remote-desktop/src/constants.rs" <<'RS'
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

struct TargetLifecycleEventCoalescer;

fn commit_geometry() {
    geometry_event_type();
    ApplicationWindowSetChanged;
    "TARGET_PERMISSION_REVOKED";
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

fn input_blocked_reason() {}

fn target_failure_payload() {}

fn commit_pending_media_rebind() {
    "TARGET_REBOUND";
}

fn commit_pending_media_rebind_failed() {
    "TARGET_REBIND_FAILED";
}

fn geometry_event_type() -> &'static str {
    if moved() {
        "TARGET_MOVED"
    } else {
        "TARGET_RESIZED"
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tracker_commits_move_resize_and_lost_without_rebinding() {}

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
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session.rs" <<'RS'
struct RemoteDesktopSession {
    consent: RemoteDesktopConsentState,
}

fn new() {
    RemoteDesktopConsentState::active(consent_grant, consent_epoch);
}

fn record_target_observation() {
    let target_loss_reason = match &observation {
        TargetObservation::Lost { reason, .. } => Some(*reason),
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
        self.transport.mark_media_source_lost(epoch);
        session_events::media_source_lost(self.target.binding());
    }
    self.push_target_tracking_event(event);
}

fn push_target_tracking_event() {
    payload["transport_epoch"] = self.transport.active_epoch();
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
        && self.transport.media_transport_ready()
        && self.transport.client_media_ready()
}

fn activate_input_for_transport_epoch() {
    if !self.consent.permits_media_input() {
        return false;
    }
}

fn close() {
    self.consent.expire();
}

fn revoke_consent() {
    self.lifecycle.suspend();
    self.transport.mark_media_source_lost(epoch);
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
        assert!(!session.report_client_media_state(epoch, "stalled"));
        assert_eq!(session.state(), RemoteDesktopState::Suspended);
        assert_eq!(session.transport_state()["primary"], json!("media_source_lost"));
        assert_eq!(session.transport_state()["device_sending"], json!(false));
        assert!(events.iter().all(|event| event["event_type"] != json!("SESSION_DEGRADED")));
    }

    #[test]
    fn client_media_stall_emits_session_degraded_recovery_event() {
        assert!(session.report_client_media_state(epoch, "presenting"));
        assert!(session.report_client_media_state(epoch, "stalled"));
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
    fn pending_media_rebind_failure_rejects_session_rebinding() {}

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
    fn consent_revocation_suspends_media_and_blocks_input_activation() {
        assert!(permission_revoked_index < media_source_lost_index);
        assert!(
            !session.activate_input_for_transport_epoch(epoch),
            "revoked consent must prevent input from reactivating even with the same transport epoch"
        );
    }
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
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_transport_state.rs" <<'RS'
enum PrimaryMediaPhase {
    MediaSourceLost,
    Failed,
}

fn can_transition_primary() {
    match from {
        PrimaryMediaPhase::MediaSourceLost => matches!(to, PrimaryMediaPhase::Failed),
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

  cat >"$SANDBOX/plugins/remote-desktop/src/session_store.rs" <<'RS'
fn mark_direct_webrtc_media_ready(session_id: &str) {
    direct_webrtc_endpoint_ura(session_id);
}

fn mark_direct_webrtc_failed() {
    WebRtcFailureEventKind::TransportFailed;
    webrtc_transport_failure_context();
}

fn fail_pending_media_rebind_for_session() {}

#[cfg(test)]
mod tests {
    #[test]
    fn production_media_ready_requires_production_codec_and_sender_ready() {
        assert_eq!(view["production_readiness"]["blocked_reason"], json!("production_codec_not_negotiated"));
        assert_eq!(view["production_readiness"]["client_media_ready"], json!(false));
        assert!(session.report_client_media_state(TransportEpoch::new(1), "presenting"));
        assert_eq!(view["transport"]["production_ready"], json!(false));
        assert_eq!(view["transports"][0]["metadata"]["production_ready"], json!(false));
    }

    #[test]
    fn direct_webrtc_transport_failure_projects_recovery_context() {
        assert_eq!(event["reason_code"], json!("transport_route_unavailable"));
        assert_eq!(event["recoverability"], json!("retry_session"));
        assert_eq!(event["payload"]["failure_domain"], json!("transport"));
        assert_eq!(event["payload"]["frontend_action"], json!("retry_session"));
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_native_media.rs" <<'RS'
fn apply_pending_media_rebind() {}
fn fail_pending_media_rebind() {}

#[cfg(test)]
mod tests {
    #[test]
    fn native_media_rebind_failure_projects_typed_target_lifecycle() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_endpoint.rs" <<'RS'
fn answer(endpoint_config: EndpointConfig) {
    json!({
        "endpoint_ura": direct_webrtc_endpoint_ura(&endpoint_config.session_id),
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
    json!({
        "consent": session.consent_state().to_value(),
        "signaling": {
            "route_state": transport_route_state.clone(),
        },
        "production_readiness": {
            "blocked_reason": production_readiness_blocked_reason(session),
            "target_scope_ready": session.target_scope_ready(),
            "production_route_ready": transport_view.production_route_ready(),
            "route_readiness_blocker": transport_view.readiness_blocker(),
            "route_state": transport_route_state.clone(),
        },
    });
}

fn production_readiness_blocked_reason(session: &RemoteDesktopSession) -> Value {
    if session.production_media_ready() {
        Value::Null
    } else if !session.target_scope_ready() {
        json!("target_scope_not_ready")
    } else if !session.production_codec_negotiated() {
        json!("production_codec_not_negotiated")
    } else if !session.media_transport_ready() {
        json!("media_transport_not_ready")
    } else if !session.client_media_ready() {
        json!("client_media_not_presenting")
    } else {
        json!("production_readiness_incomplete")
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/view_device.rs" <<'RS'
fn device_capabilities_view() {
    let production_target_subjects = production_backend.supported_subjects_value();
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
            "capture_target_models": [
                "display_surface",
                "window_surface",
                "display_scoped_application_window_set"
            ],
            "reason": "native ScreenCaptureKit/VideoToolbox WebRTC backend is available for display/window/application target capture"
        },
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn device_capabilities_report_clipboard_and_file_transfer_unsupported() {}

    #[test]
    fn device_capabilities_project_native_target_subject_matrix() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/input.rs" <<'RS'
const UNSUPPORTED_INPUT_CHANNEL_TYPES: &[&str] = &["clipboard", "file_drop"];

fn unsupported_input_channel_types_value() {}

fn input_policy_for_target_snapshot() {
    let _ = policy["pointer_target"]["target_geometry_revision"];
}

fn input_policy_object() {}

enum InputTransportGuard {
    DirectWebRtc(TransportEpoch),
    DiagnosticPreview,
}

fn current_session_input_policy() {
    InputTransportGuard::DirectWebRtc(epoch);
    let snapshot = session.target_snapshot();
    let input_scope = session.target_binding().input_scope();
    if !snapshot.input_enabled() {
        return None;
    }
    let input_policy = input_policy_for_target_snapshot(base_policy.clone(), snapshot);
    input_policy_for_scope(input_policy, input_scope);
}

fn display_interactive_without_input_consent_remains_view_only() {}

fn input_policy_for_scope() {
    match scope {
        InputScope::ViewOnly => {
            disable_input_policy_key(&mut map, "keyboard_enabled");
            disable_input_policy_key(&mut map, "pointer_enabled");
        }
    }
}

fn input_policy_reject_reason() -> Option<&'static str> {
    if input_scope == Some(InputScope::ViewOnly.as_str()) && matches!(frame_type, "key" | "pointer") {
        return Some("input_scope_unsupported");
    }
    None
}

fn reject_unsupported_input_channel_frame() {}

fn validate_input_frame() {
    reject_unsupported_input_channel_frame(frame)?;
}

fn apply_input_frame_with_policy() {
    if let Some(reason) = input_policy_reject_reason(input_policy, frame.kind().as_policy_key()) {
        return InputApplyOutcome::rejected(reason);
    }
}

fn record_rejection() {
    InputRejectSample::new(
        outcome.reason.unwrap_or("input_injection_failed"),
        rejected_count,
    );
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
    fn apply_input_frame_with_policy_is_the_policy_enforcement_boundary() {
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
    fn maps_application_pointer_through_primary_window_bounds() {
        assert!(!input_policy_allows(&policy, "pointer"));
    }

    #[test]
    fn input_reject_diagnostics_are_coalesced_across_interleaved_signatures() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/target.rs" <<'RS'
enum TargetResolutionError {
    TargetIdentityAmbiguous,
    TargetMetadataIncomplete,
    TargetIdentityChanged,
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

struct AppWindowSetProof;

impl AppWindowSetProof {
    fn window_set_epoch(&self) -> u64 {
        2
    }
}

fn application_window_set_rebind_candidate() {}

fn input_scope_for_request() -> InputScopeDecision {
    match kind {
        RemoteDesktopTargetKind::Display => {
            let reason = "input_consent_required";
            InputScope::ViewOnly
        }
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
            // target-scoped keyboard/pointer dispatch is unsafe until a focus-safe validator exists.
            let reason = "target_scoped_keyboard_pointer_dispatch_unsafe";
            InputScope::ViewOnly
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
    fn application_requires_display_scoped_stable_identity() {}

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
    let workflow = RemoteDesktopSessionCreationWorkflow::start(&env, &args)?
        .consume_consent(&registry, &env)?
        .resolve_target()?;
    let session = RemoteDesktopSession::new(workflow.into_session_init()?);
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
    current_session_input_policy(
        session_store,
        session_id,
        InputTransportGuard::DiagnosticPreview,
        input_policy,
    );
    handle_parsed_bidi_input_frame(&effective_input_policy, &frame);
}

fn handle_bidi_input_frame() {
    apply_input_frame_with_policy(input_policy, frame);
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostic_bidi_view_only_input_reports_scope_unsupported() {}

    #[test]
    fn diagnostic_bidi_input_rechecks_session_target_snapshot() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs" <<'RS'
fn select_window_for_binding() {
    Err(TargetResolutionError::TargetIdentityAmbiguous(
        "requested ScreenCaptureKit window identity is ambiguous",
    ))
}

fn select_application_for_binding() {
    Err(TargetResolutionError::TargetIdentityAmbiguous(
        "requested ScreenCaptureKit application identity is ambiguous",
    ))
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/request.rs" <<'RS'
fn request_projection() {
    unsupported_input_channel_types_value();
}

#[test]
fn input_policy_reports_clipboard_and_file_drop_unsupported_even_when_requested() {}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/target_observer.rs" <<'RS'
fn observe_bound_session_target_once() {
    let Some(inputs) = sessions.target_observation_inputs_for_session(session_id) else {
        return TargetObservationPollResult::stop_tracking();
    };
    record_target_observation_for_session();
}

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
    if &current_window_set != committed_window_set {
        return Some(TargetObservation::ApplicationWindowSetChanged {});
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
    fn snapshot_observer_reappearance_requires_explicit_rebind_policy() {}

    #[test]
    fn unsupported_platform_observer_fails_app_window_targets_closed() {}
}

#[cfg(not(target_os = "macos"))]
mod platform {
    fn sample_platform_target_observations() -> PlatformTargetObservationSample {
        PlatformTargetObservationSample::unsupported_platform()
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/lease_monitor.rs" <<'RS'
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

#[cfg(test)]
mod tests {
    #[test]
    fn target_monitor_command_state_machine_tracks_cancels_and_shuts_down() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/runtime.rs" <<'RS'
struct RemoteDesktopRuntime {
    lease_monitor: RemoteDesktopLeaseMonitor,
    target_monitor: RemoteDesktopTargetMonitor,
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
perl -0pi -e 's/unsupported_platform_target_observation/unsupported_platform_noop_observation/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'target observer must centralize unsupported platform app/window fail-closed semantics'

write_fixture
perl -0pi -e 's/PlatformTargetObservationSample::unsupported_platform\(\)/PlatformTargetObservationSample::noop()/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'non-macOS platform target sample must fail app/window targets closed instead of silently returning no observation'

write_fixture
perl -0pi -e 's/unsupported_platform_observer_fails_app_window_targets_closed/unsupported_platform_observer_silently_noops/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'target observer tests must prove unsupported platforms fail app/window targets closed'

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
perl -0pi -e 's/ApplicationWindowSetChanged/ApplicationWindowSetUnchecked/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'application observer must report same-app window-set drift as a rebind observation'

write_fixture
perl -0pi -e 's/AppWindowSetProof::new/AppWindowSetProof::unchecked/g' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'application observer must rederive the current display-scoped app window-set proof'

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
perl -0pi -e 's/pending_media_rebind_failure_rejects_session_rebinding/pending_media_rebind_failure_leaves_session_rebinding/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'session aggregate must reject Rebinding when pending media source rebuild fails'

write_fixture
perl -0pi -e 's/fail_pending_media_rebind_for_session/native_rebind_error_bridge_removed/' \
  "$SANDBOX/plugins/remote-desktop/src/session_store.rs"
run_fail 'session store must expose a target-lifecycle failure projection for native pending media rebind failures'

write_fixture
perl -0pi -e 's/native_media_rebind_failure_projects_typed_target_lifecycle/native_media_rebind_failure_projects_transport_only/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_native_media.rs"
run_fail 'native WebRTC media path must test target-lifecycle projection for pending media rebind failures'

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
perl -0pi -e 's/direct_webrtc_transport_failure_projects_recovery_context/direct_webrtc_transport_failure_lacks_recovery_context/' \
  "$SANDBOX/plugins/remote-desktop/src/session_store.rs"
run_fail 'session-store tests must prove default transport failures publish recovery context'

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
perl -0pi -e 's/"blocked_reason": production_readiness_blocked_reason\(session\),//' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'public production readiness must expose one typed blocked_reason instead of forcing UI inference'

write_fixture
perl -0pi -e 's/"client_media_not_presenting"/"production_readiness_incomplete"/' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'production readiness must distinguish missing client presenting/decoded evidence'

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
perl -0pi -e 's/PrimaryMediaPhase::MediaSourceLost => [^\n]+/PrimaryMediaPhase::MediaSourceLost => false/' \
  "$SANDBOX/plugins/remote-desktop/src/session_transport_state.rs"
run_fail 'media source loss must be absorbing until failure or a new epoch'

write_fixture
perl -0pi -e 's/let snapshot = session\.target_snapshot\(\);/let snapshot = cached_snapshot;/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'production input path must read the latest session target snapshot'

write_fixture
perl -0pi -e 's/let input_scope = session\.target_binding\(\)\.input_scope\(\);/let input_scope = cached_input_scope;/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'production input path must read input scope from the session-owned target binding'

write_fixture
perl -0pi -e 's/input_policy_for_scope\(input_policy, input_scope\);/input_policy;/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'production input path must reapply input scope after deriving latest pointer target geometry'

write_fixture
perl -0pi -e 's/current_session_input_policy_uses_same_geometry_revision_as_target_event/current_session_input_policy_allows_stale_geometry_revision/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'E2E-08 must prove target event and input mapping consume the same committed geometry revision'

write_fixture
perl -0pi -e 's/if let Some\(reason\) = input_policy_reject_reason\(input_policy, frame\.kind\(\)\.as_policy_key\(\)\) \{\n        return InputApplyOutcome::rejected\(reason\);\n    \}//' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'input frame application must enforce centralized input policy before OS injection'

write_fixture
perl -0pi -e 's/apply_input_frame_with_policy_is_the_policy_enforcement_boundary/apply_input_frame_policy_boundary_missing/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'input tests must prove apply_input_frame_with_policy is the policy enforcement boundary'

write_fixture
perl -0pi -e 's/reject_unsupported_input_channel_frame\(frame\)\?;//' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'input frame validation must reject unsupported rich input before policy application'

write_fixture
perl -0pi -e 's/parse_input_frame_rejects_clipboard_and_file_drop_before_policy_application/parse_input_frame_accepts_rich_input/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'input parser tests must prove clipboard/file-drop fail before policy application'

write_fixture
perl -0pi -e 's/current_session_input_policy/static_input_policy/' \
  "$SANDBOX/plugins/remote-desktop/src/invoke_bidi.rs"
run_fail 'diagnostic bidi input path must re-read session readiness for each input frame'

write_fixture
perl -0pi -e 's/apply_input_frame_with_policy\(input_policy, frame\)/input_policy_reject_reason(input_policy, kind)/' \
  "$SANDBOX/plugins/remote-desktop/src/invoke_bidi.rs"
run_fail 'diagnostic bidi input path must use the single policy-enforced input application boundary'

write_fixture
perl -0pi -e 's/json!\(false\)/json!(true)/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'E2E-09 must assert target loss disables input'

write_fixture
perl -0pi -e 's/TargetResolutionError::TargetIdentityAmbiguous/TargetResolutionError::TargetNotFound/g' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"
run_fail 'ScreenCaptureKit binding must fail closed on native identity ambiguity'

write_fixture
perl -0pi -e 's/target_identity_ambiguous/target_metadata_incomplete/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"
run_fail 'weak target identity handler test must assert target_identity_ambiguous'

write_fixture
perl -0pi -e 's/disable_input_policy_key\(&mut map, "pointer_enabled"\);/enable_input_policy_key(&mut map, "pointer_enabled");/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'view-only input policy must disable pointer'

write_fixture
perl -0pi -e 's/Some\("input_scope_unsupported"\)/Some("input_disabled")/' \
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
perl -0pi -e 's/production_backend\.supported_subjects_value\(\)/json!(["display"])/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must project production target subjects from the backend descriptor'

write_fixture
perl -0pi -e 's/"production_target_subjects": production_target_subjects,//' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose the native production backend display/window/application subject matrix'

write_fixture
perl -0pi -e 's/"display_scoped_application_window_set"/"application"/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must expose the application target model instead of flattening applications to display capture'

write_fixture
perl -0pi -e 's#display/window/application target capture#display capture#' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must describe native ScreenCaptureKit as targeted display/window/application capture'

write_fixture
perl -0pi -e 's/device_capabilities_project_native_target_subject_matrix/device_capabilities_hide_native_target_subject_matrix/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capability tests must prove the native target subject matrix is projected'

write_fixture
perl -0pi -e 's/unsupported_input_channel_types_value\(\);//' \
  "$SANDBOX/plugins/remote-desktop/src/request.rs"
run_fail 'request input policy projection must reuse the input-domain unsupported type set'

printf 'test_check_remoteapp_lifecycle_input_boundary.sh: all cases passed\n'
