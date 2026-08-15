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

fn commit_geometry() {
    geometry_event_type();
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

fn input_blocked_reason() {}

fn target_failure_payload() {}

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
        assert_eq!(event["event_type"], json!("TARGET_REBIND_FAILED"));
    }

    #[test]
    fn production_media_ready_requires_target_scope_ready() {
        assert!(
            !session.production_media_ready(),
            "scope widening or display fallback must prevent production online"
        );
        assert_eq!(target_bound["payload"]["display_fallback_used"], json!(true));
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
        "preview_ability": "screen.subscribe",
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
        "reason_code": "capture_target_resolved",
        "recoverability": "continue",
        "target_binding": binding.to_value(),
        "scope_audit": binding.scope_audit_value(),
    });
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
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_media.rs" <<'RS'
fn direct_webrtc_target_failure_projection() {
    WebRtcFailureEventKind::MediaSourceLost;
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/event_log.rs" <<'RS'
fn event_type_proto_name(event_type: &str) -> &'static str {
    match event_type {
        "TARGET_REBIND_FAILED" => "REMOTE_DESKTOP_EVENT_TARGET_CHANGED",
        _ => "REMOTE_DESKTOP_EVENT_STATE_CHANGED",
    }
}
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
    fn summary() {
        "message": self.message,
        "reason_code": self.reason_code.clone(),
        "metadata": {
            "input_channel_label": INPUT_DATA_CHANNEL_LABEL,
            "reason_code": self.reason_code.clone(),
        };
    }
    json!({
        "host_candidate": true,
        "stun_srflx": false,
        "turn_relay": false,
        "easynet_relay": false,
        "failed": false,
        "production_ready": session.production_media_ready(),
    });
    fn transport_reason_code() {
        TargetResolutionError::TransportRouteUnavailable;
        "transport_route_unavailable";
        RemoteDesktopTransportBlocker::from_webrtc_error;
    }
    fn transport_route_failed() {}
    "host_only_no_nat_or_relay";
    "relay_unavailable";
}

fn direct_endpoint_ura(session: &RemoteDesktopSession) {
    direct_webrtc_endpoint_ura(session.session_id());
}

#[test]
fn host_only_candidates_are_not_reported_as_nat_or_relay_ready() {}

#[test]
fn easynet_relay_does_not_imply_turn_relay() {}

#[test]
fn srflx_without_relay_reports_typed_relay_unavailable_reason() {
    assert_eq!(summary["reason_code"], json!("transport_route_unavailable"));
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_store.rs" <<'RS'
fn mark_direct_webrtc_media_ready(session_id: &str) {
    direct_webrtc_endpoint_ura(session_id);
}

fn mark_direct_webrtc_failed() {
    WebRtcFailureEventKind::TransportFailed;
    webrtc_transport_failure_context();
}

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

  cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_endpoint.rs" <<'RS'
fn answer(endpoint_config: EndpointConfig) {
    json!({
        "endpoint_ura": direct_webrtc_endpoint_ura(&endpoint_config.session_id),
    });
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_negotiation.rs" <<'RS'
fn commit_started_endpoint(session_id: &str) {
    direct_webrtc_endpoint_ura(session_id);
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
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn device_capabilities_report_clipboard_and_file_transfer_unsupported() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/input.rs" <<'RS'
const UNSUPPORTED_INPUT_CHANNEL_TYPES: &[&str] = &["clipboard", "file_drop"];

fn unsupported_input_channel_types_value() {}

fn input_policy_for_target_snapshot() {
    let _ = policy["pointer_target"]["target_geometry_revision"];
}

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

fn input_policy_for_scope() {
    match scope {
        InputScope::ViewOnly => {
            disable_input_policy_key(map, "keyboard_enabled");
            disable_input_policy_key(map, "pointer_enabled");
        }
    }
}

fn input_policy_reject_reason() -> Option<&'static str> {
    if input_scope == Some(InputScope::ViewOnly.as_str()) && matches!(frame_type, "key" | "pointer") {
        return Some("input_scope_unsupported");
    }
    None
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
    fn current_session_input_policy_uses_same_geometry_revision_as_target_event() {}

    #[test]
    fn apply_input_frame_with_policy_is_the_policy_enforcement_boundary() {
        assert_eq!(outcome.reason, Some("input_policy_denied"));
        assert_eq!(view_only_pointer.reason, Some("input_scope_unsupported"));
        assert_eq!(view_only_key.reason, Some("input_scope_unsupported"));
        assert_eq!(clipboard_outcome.reason, Some("clipboard_input_unsupported"));
    }

    #[test]
    fn maps_window_relative_pointer_to_global_screen_point() {
        assert!(!input_policy_allows(&policy, "pointer"));
    }

    #[test]
    fn maps_application_pointer_through_primary_window_bounds() {
        assert!(!input_policy_allows(&policy, "pointer"));
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/target.rs" <<'RS'
enum TargetResolutionError {
    TargetIdentityAmbiguous,
    TargetMetadataIncomplete,
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
        "input_scope_reason": self.scope_audit.input_scope_reason.as_str(),
        "scope_widened": self.scope_audit.scope_widened,
        "display_fallback_used": self.scope_audit.display_fallback_used,
    });
}

struct InputScopeDecision;

fn input_scope_for_request() -> InputScopeDecision {
    match kind {
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
            // target-scoped keyboard/pointer dispatch is unsafe until a focus-safe validator exists.
            let reason = "target_scoped_keyboard_pointer_dispatch_unsafe";
            InputScope::ViewOnly
        }
        _ => InputScope::DisplayGlobal,
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
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs" <<'RS'
fn handle() {
    let workflow = RemoteDesktopSessionCreationWorkflow::start(&env, &args)?
        .consume_consent(&registry, &env)?
        .resolve_target()?;
    let session = RemoteDesktopSession::new(workflow.into_session_init());
    RemoteDesktopPlugin::track_session_target(&plugin, tracker_session_id);
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
    record_target_observation_for_session();
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

#[cfg(test)]
mod tests {
    #[test]
    fn observation_provider_commits_through_session_store_boundary() {}

    #[test]
    fn stale_observation_cannot_commit_after_session_binding_reuse() {}

    #[test]
    fn lost_observation_returns_media_source_stop_effect_after_debounce() {}

    #[test]
    fn window_observation_prioritizes_visibility_loss_over_title_or_focus_changes() {}

    #[test]
    fn snapshot_observer_reappearance_requires_explicit_rebind_policy() {}
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

#[cfg(test)]
mod tests {
    #[test]
    fn target_monitor_command_state_machine_tracks_cancels_and_shuts_down() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/runtime.rs" <<'RS'
struct RemoteDesktopRuntime {
    target_monitor: RemoteDesktopTargetMonitor,
}

fn track_session_target(plugin: &Arc<RemoteDesktopPlugin>, session_id: String) {
    plugin.target_monitor.track(plugin, session_id);
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
perl -0pi -e 's/direct_webrtc_endpoint_ura\(session\.session_id\(\)\)/legacy_endpoint(session.session_id())/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'public transport view must derive endpoint_ura from the canonical direct WebRTC endpoint helper'

write_fixture
perl -0pi -e 's#"relay_unavailable";#"relay_unavailable"; "webrtc://direct/legacy";#' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'remote desktop endpoint_ura evidence must be EasyNet URA only'

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
perl -0pi -e 's/RemoteDesktopPlugin::track_session_target\(&plugin, tracker_session_id\);//' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"
run_fail 'create_session must register created sessions with the target monitor'

write_fixture
perl -0pi -e 's/plugin\.cancel_session_target_tracking\(session_id\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session_lifecycle.rs"
run_fail 'terminal session cleanup must cancel target tracking'

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
perl -0pi -e 's/snapshot_observer_reappearance_requires_explicit_rebind_policy/snapshot_observer_reappearance_revives_stale_media/' \
  "$SANDBOX/plugins/remote-desktop/src/target_observer.rs"
run_fail 'target observer must prove platform-visible target reappearance cannot revive media/input without explicit rebind policy'

write_fixture
perl -0pi -e 's/"TARGET_REBIND_FAILED" => "REMOTE_DESKTOP_EVENT_TARGET_CHANGED"/"TARGET_REBIND_FAILED" => "REMOTE_DESKTOP_EVENT_STATE_CHANGED"/' \
  "$SANDBOX/plugins/remote-desktop/src/event_log.rs"
run_fail 'event log must project TARGET_REBIND_FAILED as a canonical target change'

write_fixture
perl -0pi -e 's/session_events::media_source_lost\(self\.target\.binding\(\)\)/session_events::media_source_lost()/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'MEDIA_SOURCE_LOST projection must consume the committed session target binding'

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
perl -0pi -e 's/"binding_id": binding\.binding_id\(\),//' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'CAPTURE_TARGET_RESOLVED path must publish initial binding context and continue recoverability'

write_fixture
perl -0pi -e 's/capture_target_resolved_payload_projects_initial_binding_context/capture_target_resolved_lacks_initial_binding_context/' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'session event tests must prove CAPTURE_TARGET_RESOLVED publishes binding context'

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
perl -0pi -e 's/fn transport_reason_code/fn transport_detail_reason/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport route reason_code derivation must be centralized in the transport view projection'

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
perl -0pi -e 's/\n    } else if !session\.client_media_ready\(\) \{\n        json!\("client_media_not_presenting"\)//' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'production readiness must distinguish missing client presenting/decoded evidence'

write_fixture
perl -0pi -e 's/"production_ready": session\.production_media_ready\(\),//' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport projection must expose production_ready from the session production predicate'

write_fixture
perl -0pi -e 's/"display_fallback_used": self\.scope_audit\.display_fallback_used/"display_fallback_used": false/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"
run_fail 'TARGET_BOUND payload must project display fallback from the committed binding audit'

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
perl -0pi -e 's/disable_input_policy_key\(map, "pointer_enabled"\);/enable_input_policy_key(map, "pointer_enabled");/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'view-only input policy must disable pointer'

write_fixture
perl -0pi -e 's/Some\("input_scope_unsupported"\)/Some("input_disabled")/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'view-only key/pointer rejection must report input_scope_unsupported'

write_fixture
perl -0pi -e 's/"unsupported_input_types": unsupported_input_channel_types_value\(\),/"supported_input_types": unsupported_input_channel_types_value(),/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must report unsupported input channel types'

write_fixture
perl -0pi -e 's/"unsupported_capabilities":/"enabled_capabilities":/' \
  "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must report unsupported rich-input capabilities'

write_fixture
perl -0pi -e 's/unsupported_input_channel_types_value\(\);//' \
  "$SANDBOX/plugins/remote-desktop/src/request.rs"
run_fail 'request input policy projection must reuse the input-domain unsupported type set'

printf 'test_check_remoteapp_lifecycle_input_boundary.sh: all cases passed\n'
