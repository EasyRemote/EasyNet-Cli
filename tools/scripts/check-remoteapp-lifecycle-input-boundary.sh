#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_LIFECYCLE_INPUT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
REMOTE_ROOT="$ROOT/plugins/remote-desktop/src"
SPEC="$ROOT/docs/design/remoteapp-targeted-session-spec.md"

fail() {
  printf 'check-remoteapp-lifecycle-input-boundary: %s\n' "$1" >&2
  exit 1
}

require() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  rg -q -- "$pattern" "$path" || fail "$message"
}

require_multiline() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  perl -0ne "exit(($pattern) ? 0 : 1)" "$path" || fail "$message"
}

reject() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  if rg -q -- "$pattern" "$path"; then
    fail "$message"
  fi
}

reject_multiline() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  if perl -0ne "exit(($pattern) ? 0 : 1)" "$path"; then
    fail "$message"
  fi
}

[[ -d "$REMOTE_ROOT" ]] || fail "missing remote desktop source root"
[[ -f "$SPEC" ]] || fail "missing remoteapp targeted session SPEC"

TARGET_TRACKING="$REMOTE_ROOT/target_tracking.rs"
TARGET_OBSERVER="$REMOTE_ROOT/target_observer.rs"
SESSION="$REMOTE_ROOT/session.rs"
CONTRACT="$REMOTE_ROOT/contract.rs"
SESSION_STATE="$REMOTE_ROOT/session_state.rs"
SESSION_TRANSPORT_STATE="$REMOTE_ROOT/session_transport_state.rs"
SESSION_EVENTS="$REMOTE_ROOT/session_events.rs"
INPUT="$REMOTE_ROOT/input.rs"
TARGET="$REMOTE_ROOT/target.rs"
SCK="$REMOTE_ROOT/screencapturekit_capture.rs"
REQUEST="$REMOTE_ROOT/request.rs"
CREATE_SESSION="$REMOTE_ROOT/handlers/create_session.rs"
SESSION_CREATION="$REMOTE_ROOT/session_creation.rs"
INVOKE_BIDI="$REMOTE_ROOT/invoke_bidi.rs"

for file in "$TARGET_TRACKING" "$TARGET_OBSERVER" "$SESSION" "$CONTRACT" "$SESSION_STATE" "$SESSION_TRANSPORT_STATE" "$SESSION_EVENTS" "$INPUT" "$TARGET" "$SCK" "$REQUEST" "$CREATE_SESSION" "$SESSION_CREATION" "$INVOKE_BIDI"; do
  [[ -f "$file" ]] || fail "missing required source ${file#"$ROOT/"}"
done

for checkpoint in \
  'E2E-08 move/resize tracking' \
  'E2E-09 target loss vs transport failure' \
  'E2E-10 weak identity ambiguity' \
  'E2E-11 view-only input safety'; do
  require "$checkpoint" "$SPEC" "SPEC must retain $checkpoint acceptance"
done

# E2E-08: move/resize tracking must be a session-owned target tracker state
# machine. Geometry changes advance target_geometry_revision and input mapping
# must consume the same latest tracker snapshot.
require 'struct TargetTrackerSnapshot' "$TARGET_TRACKING" \
  'target lifecycle must have a named snapshot state object'
require 'target_geometry_revision: u64' "$TARGET_TRACKING" \
  'target tracker snapshot must carry target_geometry_revision'
require 'fn commit_geometry\(' "$TARGET_TRACKING" \
  'target tracker must own geometry commits'
require 'geometry_event_type\(' "$TARGET_TRACKING" \
  'target tracker must classify geometry changes centrally'
require '"TARGET_MOVED"' "$TARGET_TRACKING" \
  'target tracker must emit TARGET_MOVED'
require '"TARGET_RESIZED"' "$TARGET_TRACKING" \
  'target tracker must emit TARGET_RESIZED'
require 'tracker_commits_move_resize_and_lost_without_rebinding' "$TARGET_TRACKING" \
  'E2E-08 must have move/resize/lost tracker regression coverage'
require 'pointer_policy_consumes_latest_target_tracker_snapshot' "$INPUT" \
  'E2E-08 input transform must test latest target tracker snapshot consumption'
require 'input_policy_for_target_snapshot\(' "$INPUT" \
  'input policy must be derivable from the latest target tracker snapshot'
require 'fn current_session_input_policy\(' "$INPUT" \
  'production input path must resolve current policy per frame'
require 'let snapshot = session\.target_snapshot\(\);' "$INPUT" \
  'production input path must read the latest session target snapshot'
require 'if !snapshot\.input_enabled\(\)' "$INPUT" \
  'production input path must disable input after target loss'
require 'policy\["pointer_target"\]\["target_geometry_revision"\]' "$INPUT" \
  'input policy test must assert pointer target geometry revision'
require 'observe_bound_session_target_once' "$TARGET_OBSERVER" \
  'target observer must expose the bound-session observation boundary'
require 'record_target_observation_for_session' "$TARGET_OBSERVER" \
  'target observer must commit through the session store boundary'
require 'observation_provider_commits_through_session_store_boundary' "$TARGET_OBSERVER" \
  'E2E-08 must test observer-to-session-store geometry commits'
require 'stale_observation_cannot_commit_after_session_binding_reuse' "$TARGET_OBSERVER" \
  'stale observations must not advance a reused session binding'

# E2E-09: target loss is not transport failure. The session aggregate must
# degrade the media source, disable input, and project MEDIA_SOURCE_LOST with
# failure_domain=target after TARGET_LOST.
require 'fn record_target_observation\(' "$SESSION" \
  'session aggregate must be the committed target observation writer'
require 'event\.event_type\(\) == "TARGET_LOST"' "$SESSION" \
  'session must branch target loss separately from transport failure'
require 'Suspended,' "$CONTRACT" \
  'remote desktop lifecycle must expose an explicit Suspended state'
require 'REMOTE_DESKTOP_SESSION_STATE_SUSPENDED' "$CONTRACT" \
  'remote desktop lifecycle must expose a canonical Suspended wire state'
require 'fn mark_suspended\(' "$SESSION_STATE" \
  'session state machine must centralize target-loss suspension'
require 'self\.lifecycle\.mark_suspended\(\)' "$SESSION" \
  'target loss must suspend the session lifecycle'
reject_multiline 'm/event\.event_type\(\) == "TARGET_LOST".{0,240}?mark_degraded\(\)/s' "$SESSION" \
  'target loss must not reuse transport degraded lifecycle semantics'
require 'fn can_transition_primary\(' "$SESSION_TRANSPORT_STATE" \
  'transport phase transitions must be centralized'
require 'PrimaryMediaPhase::MediaSourceLost => matches!\(to, PrimaryMediaPhase::Failed\)' "$SESSION_TRANSPORT_STATE" \
  'media source loss must be absorbing until failure or a new epoch'
require 'media_source_lost_is_absorbing_until_new_epoch_or_failure' "$SESSION_TRANSPORT_STATE" \
  'transport tests must prove media source loss cannot reopen readiness in the same epoch'
require 'self\.transport\.mark_media_source_lost\(epoch\)' "$SESSION" \
  'target loss must stop the active media source'
require 'session_events::media_source_lost' "$SESSION" \
  'target loss must project a media-source lost event'
require 'target_lost_stops_active_media_source_without_transport_failure' "$SESSION" \
  'E2E-09 must test target loss versus transport failure'
require 'target_lost_index < media_source_lost_index' "$SESSION" \
  'E2E-09 must prove TARGET_LOST is ordered before MEDIA_SOURCE_LOST'
require 'RemoteDesktopState::Suspended' "$SESSION" \
  'E2E-09 must assert target loss moves the session to suspended'
require 'REMOTE_DESKTOP_SESSION_STATE_SUSPENDED' "$SESSION" \
  'E2E-09 must assert suspended state is projected to events'
require_multiline 'm/session\.target_tracking_state\(\)\["input_enabled"\]\s*,\s*json!\(false\)/s' "$SESSION" \
  'E2E-09 must assert target loss disables input'
require 'target_loss_rejects_late_client_media_state_without_degrading_session' "$SESSION" \
  'E2E-09 must test late client media state cannot degrade a suspended target-loss session'
require 'report_client_media_state\(epoch, "stalled"\)' "$SESSION" \
  'E2E-09 must exercise the late client media-state path after target loss'
require 'lost_observation_returns_media_source_stop_effect_after_debounce' "$TARGET_OBSERVER" \
  'E2E-09 must test observer-debounced target loss returns media stop effect'
require 'MEDIA_SOURCE_LOST' "$SESSION_EVENTS" \
  'session events must project MEDIA_SOURCE_LOST'
require '"failure_domain": "target"' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST must be a target-domain failure'
require '"media_transport_ready": false' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST must mark media transport unavailable'
reject 'TRANSPORT_FAILED' "$SESSION" \
  'session target-loss tests must not rely on a transport-failed event'

# E2E-10: weak identity must fail closed as ambiguity or metadata-incomplete
# before any stream starts. Native ScreenCaptureKit selection must also return
# target_identity_ambiguous for multiple stable-identity matches.
require 'TargetResolutionError::TargetIdentityAmbiguous' "$TARGET" \
  'target taxonomy must include target_identity_ambiguous'
require 'TargetResolutionError::TargetMetadataIncomplete' "$TARGET" \
  'target taxonomy must retain metadata-incomplete for truly missing proof fields'
require 'window targets require owner pid, app_identity, or bundle_id' "$TARGET" \
  'window targets must reject title/app-name-only identity'
require 'application targets require primary_pid, app_identity, or bundle_id' "$TARGET" \
  'application targets must reject app-name-only identity'
require 'target_identity_ambiguous' "$TARGET" \
  'target ambiguity reason must be externally visible'
require 'window_requires_stable_owner_identity_not_app_name_only' "$TARGET" \
  'weak window identity must have resolver regression coverage'
require 'application_requires_display_scoped_stable_identity' "$TARGET" \
  'weak application identity must have resolver regression coverage'
require 'create_session_rejects_weak_window_identity_before_session_insert' "$CREATE_SESSION" \
  'weak target identity must fail before session insertion'
require 'target_identity_ambiguous' "$CREATE_SESSION" \
  'weak target identity handler test must assert target_identity_ambiguous'
require '!sessions\.contains_key\("rd-weak-window"\)' "$CREATE_SESSION" \
  'weak target identity failure must not insert a session row'
require 'RemoteDesktopSessionCreationWorkflow::start' "$CREATE_SESSION" \
  'create_session must use the pre-row creation workflow'
require '\.resolve_target\(\)\?' "$CREATE_SESSION" \
  'create_session must resolve target before constructing an active session'
require 'RemoteDesktopSession::new\(workflow\.into_session_init\(\)\)' "$CREATE_SESSION" \
  'create_session must construct the session only after target resolution'
require 'RemoteDesktopSessionCreationState::ReadyToInsert' "$SESSION_CREATION" \
  'creation workflow must have an explicit ready-to-insert state'
require 'TargetResolutionError::TargetIdentityAmbiguous' "$SCK" \
  'ScreenCaptureKit binding must fail closed on native identity ambiguity'
require 'requested ScreenCaptureKit window identity is ambiguous' "$SCK" \
  'window selection ambiguity must be typed'
require 'requested ScreenCaptureKit application identity is ambiguous' "$SCK" \
  'application selection ambiguity must be typed'

# E2E-11: app/window input remains view-only until a focus-safe target-local
# input validator exists. Pointer geometry may be computed for diagnostics, but
# keyboard/pointer injection is disabled for app/window sessions.
require 'fn input_scope_for_request\(' "$TARGET" \
  'target resolver must centralize requested mode to input scope'
require 'RemoteDesktopTargetKind::Window \| RemoteDesktopTargetKind::Application' "$TARGET" \
  'window/application targets must share the view-only input safety branch'
require 'InputScope::ViewOnly' "$TARGET" \
  'app/window requested interactive mode must downgrade to view_only'
require 'target-scoped keyboard/pointer dispatch is unsafe' "$TARGET" \
  'view-only downgrade must document the missing focus-safe validator'
require 'fn input_policy_for_scope\(' "$INPUT" \
  'input policy must centralize scope-based disablement'
require 'fn input_policy_reject_reason\(' "$INPUT" \
  'input rejection reason must be centralized for datachannel and bidi paths'
require 'InputScope::ViewOnly => \{' "$INPUT" \
  'view-only input policy branch must exist'
require 'disable_input_policy_key\(map, "keyboard_enabled"\)' "$INPUT" \
  'view-only input policy must disable keyboard'
require 'disable_input_policy_key\(map, "pointer_enabled"\)' "$INPUT" \
  'view-only input policy must disable pointer'
require 'Some\("input_scope_unsupported"\)' "$INPUT" \
  'view-only key/pointer rejection must report input_scope_unsupported'
require 'InputRejectSample::new\(reason, rejected_count\)' "$INPUT" \
  'WebRTC input rejection diagnostics must use the centralized reject reason'
require 'fn current_session_input_policy\(' "$INPUT" \
  'input readiness must be centralized at the session aggregate boundary'
require 'InputTransportGuard::DirectWebRtc\(epoch\)' "$INPUT" \
  'production input path must guard frames by the current WebRTC transport epoch'
require 'current_session_input_policy\(' "$INVOKE_BIDI" \
  'diagnostic bidi input path must re-read session readiness for each input frame'
require 'InputTransportGuard::DiagnosticPreview' "$INVOKE_BIDI" \
  'diagnostic bidi input path must guard frames by preview attachment state'
require 'handle_bidi_input_frame\(&effective_input_policy' "$INVOKE_BIDI" \
  'diagnostic bidi input path must apply frame parsing/injection against refreshed policy'
require 'input_policy_reject_reason\(input_policy, kind\)' "$INVOKE_BIDI" \
  'diagnostic bidi input path must use the same reject reason contract'
require 'diagnostic_bidi_view_only_input_reports_scope_unsupported' "$INVOKE_BIDI" \
  'E2E-11 must test bidi view-only input_scope_unsupported reporting'
require 'diagnostic_bidi_input_rechecks_session_target_snapshot' "$INVOKE_BIDI" \
  'diagnostic bidi path must fail closed after session target loss'
require 'maps_window_relative_pointer_to_global_screen_point' "$INPUT" \
  'E2E-11 must test window pointer mapping remains view-only'
require 'maps_application_pointer_through_primary_window_bounds' "$INPUT" \
  'E2E-11 must test application pointer mapping remains view-only'
require '!input_policy_allows\(&policy, "pointer"\)' "$INPUT" \
  'E2E-11 tests must assert app/window pointer input is disabled'
require 'input_policy_reports_clipboard_and_file_drop_unsupported_even_when_requested' "$REQUEST" \
  'input parser must report unsupported rich input types explicitly'

printf 'check-remoteapp-lifecycle-input-boundary: ok\n'
