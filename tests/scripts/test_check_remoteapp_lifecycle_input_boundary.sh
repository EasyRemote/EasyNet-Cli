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
    if target_lost {
        "TARGET_REBIND_FAILED";
        "explicit_rebind_required";
        json!({
            "target_status": "lost",
            "input_enabled": false,
        });
    }
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
    fn tracker_reports_rebind_failure_after_target_loss_without_policy() {}

    #[test]
    fn tracker_routes_post_loss_title_focus_through_explicit_rebind() {}
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

fn production_media_ready() -> bool {
    self.target.binding().production_scope_ready()
        && self.signaling.production_codec_negotiated()
        && self.transport.media_transport_ready()
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
        assert_eq!(events[target_lost_index]["transport_epoch"], json!(epoch.value()));
        assert_eq!(events[media_source_lost_index]["binding_id"], json!(session.target_binding().binding_id()));
        assert_eq!(events[media_source_lost_index]["target_identity_epoch"], json!(session.target_binding().target_identity_epoch()));
        assert_eq!(events[media_source_lost_index]["media_source_epoch"], json!(session.target_binding().media_source_epoch()));
        assert_eq!(session.target_tracking_state()["input_enabled"], json!(false));
    }

    #[test]
    fn target_tracking_events_include_active_transport_epoch_at_session_boundary() {
        assert_eq!(target_event["transport_epoch"], json!(epoch.value()));
        assert_eq!(target_event["payload"]["transport_epoch"], json!(epoch.value()));
    }

    #[test]
    fn target_loss_rejects_late_client_media_state_without_degrading_session() {
        assert!(!session.report_client_media_state(epoch, "stalled"));
        assert_eq!(session.state(), RemoteDesktopState::Suspended);
        assert_eq!(session.transport_state()["primary"], json!("media_source_lost"));
        assert_eq!(session.transport_state()["device_sending"], json!(false));
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

RS

  cat >"$SANDBOX/plugins/remote-desktop/src/event_log.rs" <<'RS'
fn event_type_proto_name(event_type: &str) -> &'static str {
    match event_type {
        "TARGET_REBIND_FAILED" => "REMOTE_DESKTOP_EVENT_TARGET_CHANGED",
        _ => "REMOTE_DESKTOP_EVENT_STATE_CHANGED",
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/view_transport.rs" <<'RS'
fn transport_route_state() {
    json!({
        "host_candidate": true,
        "stun_srflx": false,
        "turn_relay": false,
        "easynet_relay": false,
        "failed": false,
    });
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
fn srflx_without_relay_reports_typed_relay_unavailable_reason() {}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_store.rs" <<'RS'
fn mark_direct_webrtc_media_ready(session_id: &str) {
    direct_webrtc_endpoint_ura(session_id);
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
            "target_scope_ready": session.target_scope_ready(),
            "route_state": transport_route_state.clone(),
        },
    });
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
        "scope_widened": self.scope_audit.scope_widened,
        "display_fallback_used": self.scope_audit.display_fallback_used,
    });
}

fn input_scope_for_request() {
    match kind {
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
            // target-scoped keyboard/pointer dispatch is unsafe until a focus-safe validator exists.
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
perl -0pi -e 's/"binding_id": binding\.binding_id\(\),//' \
  "$SANDBOX/plugins/remote-desktop/src/session_events.rs"
run_fail 'MEDIA_SOURCE_LOST payload must carry binding id'

write_fixture
perl -0pi -e 's/fn transport_route_state/fn transport_summary_without_routes/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'transport view must expose host/STUN/TURN/EasyNet relay route state'

write_fixture
perl -0pi -e 's/"host_only_no_nat_or_relay"/"webrtc_ice_connecting"/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'host-only transport degradation must have a typed unavailable reason'

write_fixture
perl -0pi -e 's/"relay_unavailable"/"webrtc_ice_connecting"/' \
  "$SANDBOX/plugins/remote-desktop/src/view_transport.rs"
run_fail 'relay-unavailable transport degradation must have a typed unavailable reason'

write_fixture
perl -0pi -e 's/route_state/route_summary/g' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'public session view must project route state'

write_fixture
perl -0pi -e 's/\.suspend\(\)/.mark_degraded()/g' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'target loss must suspend the session lifecycle'

write_fixture
perl -0pi -e 's/self\.target\.binding\(\)\.production_scope_ready\(\)\s*&&\s*//' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'production media readiness must be gated by target binding scope readiness'

write_fixture
perl -0pi -e 's/"target_scope_ready": session\.target_scope_ready\(\),//' \
  "$SANDBOX/plugins/remote-desktop/src/view.rs"
run_fail 'public production readiness must expose target scope readiness'

write_fixture
perl -0pi -e 's/"display_fallback_used": self\.scope_audit\.display_fallback_used/"display_fallback_used": false/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"
run_fail 'TARGET_BOUND payload must project display fallback from the committed binding audit'

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
