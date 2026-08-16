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
TARGET_MONITOR="$REMOTE_ROOT/target_monitor.rs"
LEASE_MONITOR="$REMOTE_ROOT/lease_monitor.rs"
SESSION="$REMOTE_ROOT/session.rs"
SESSION_CONSENT_STATE="$REMOTE_ROOT/session_consent_state.rs"
SESSION_IDENTITY="$REMOTE_ROOT/session_identity.rs"
RUNTIME="$REMOTE_ROOT/runtime.rs"
CONTRACT="$REMOTE_ROOT/contract.rs"
SESSION_STATE="$REMOTE_ROOT/session_state.rs"
SESSION_TRANSPORT_STATE="$REMOTE_ROOT/session_transport_state.rs"
SESSION_EVENTS="$REMOTE_ROOT/session_events.rs"
EVENT_LOG="$REMOTE_ROOT/event_log.rs"
VIEW_TRANSPORT="$REMOTE_ROOT/view_transport.rs"
VIEW="$REMOTE_ROOT/view.rs"
VIEW_DEVICE="$REMOTE_ROOT/view_device.rs"
INPUT="$REMOTE_ROOT/input.rs"
TARGET="$REMOTE_ROOT/target.rs"
CONSTANTS="$REMOTE_ROOT/constants.rs"
SCK="$REMOTE_ROOT/screencapturekit_capture.rs"
REQUEST="$REMOTE_ROOT/request.rs"
SESSION_STORE="$REMOTE_ROOT/session_store.rs"
CREATE_SESSION="$REMOTE_ROOT/handlers/create_session.rs"
SET_DESCRIPTION="$REMOTE_ROOT/handlers/set_description.rs"
SESSION_LIFECYCLE="$REMOTE_ROOT/session_lifecycle.rs"
SESSION_CREATION="$REMOTE_ROOT/session_creation.rs"
INVOKE_BIDI="$REMOTE_ROOT/invoke_bidi.rs"
WEBRTC_ENDPOINT="$REMOTE_ROOT/transport/webrtc_endpoint.rs"
WEBRTC_MEDIA="$REMOTE_ROOT/transport/webrtc_media.rs"
WEBRTC_NEGOTIATION="$REMOTE_ROOT/transport/webrtc_negotiation.rs"
TRANSPORT_BLOCKER="$REMOTE_ROOT/transport_blocker.rs"

for file in "$TARGET_TRACKING" "$TARGET_OBSERVER" "$TARGET_MONITOR" "$LEASE_MONITOR" "$SESSION" "$SESSION_CONSENT_STATE" "$SESSION_IDENTITY" "$RUNTIME" "$CONTRACT" "$SESSION_STATE" "$SESSION_TRANSPORT_STATE" "$SESSION_EVENTS" "$EVENT_LOG" "$VIEW_TRANSPORT" "$VIEW" "$VIEW_DEVICE" "$INPUT" "$TARGET" "$CONSTANTS" "$SCK" "$REQUEST" "$SESSION_STORE" "$CREATE_SESSION" "$SET_DESCRIPTION" "$SESSION_LIFECYCLE" "$SESSION_CREATION" "$INVOKE_BIDI" "$WEBRTC_ENDPOINT" "$WEBRTC_MEDIA" "$WEBRTC_NEGOTIATION" "$TRANSPORT_BLOCKER"; do
  [[ -f "$file" ]] || fail "missing required source ${file#"$ROOT/"}"
done

require 'target_field\("consent_epoch"\)' "$EVENT_LOG" \
  'event log must lift consent epoch from target event payloads'
require '"consent_epoch": consent_epoch' "$EVENT_LOG" \
  'watch-events rows must project consent epoch as a top-level field'
require '"consent_epoch": Value::Null' "$EVENT_LOG" \
  'event-log compaction marker must preserve the consent epoch field shape'

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
require 'TARGET_LIFECYCLE_EVENT_COALESCE_INTERVAL_MS: u64 = 100' "$TARGET_TRACKING" \
  'move/resize/title event coalescing must be capped at 10Hz per session'
require 'struct TargetLifecycleEventCoalescer' "$TARGET_TRACKING" \
  'target lifecycle event coalescing must live in the session target state machine domain'
require 'fn coalesced_lifecycle_event\(' "$TARGET_TRACKING" \
  'move/resize/title events must pass through one session-level coalescing boundary'
require '"coalesced_target_events"' "$TARGET_TRACKING" \
  'coalesced target lifecycle events must report suppressed event count'
require 'tracker_coalesces_high_rate_geometry_and_title_events' "$TARGET_TRACKING" \
  'SPEC max 10Hz move/resize/title coalescing must have alternating-event regression coverage'
require 'tracker_commits_move_resize_and_lost_without_rebinding' "$TARGET_TRACKING" \
  'E2E-08 must have move/resize/lost tracker regression coverage'
require 'pointer_policy_consumes_latest_target_tracker_snapshot' "$INPUT" \
  'E2E-08 input transform must test latest target tracker snapshot consumption'
require 'current_session_input_policy_reapplies_session_input_scope_to_latest_snapshot' "$INPUT" \
  'production input path must test reapplying session-owned input scope to the latest target snapshot'
require 'current_session_input_policy_uses_same_geometry_revision_as_target_event' "$INPUT" \
  'E2E-08 must prove target event and input mapping consume the same committed geometry revision'
require 'input_policy_for_target_snapshot\(' "$INPUT" \
  'input policy must be derivable from the latest target tracker snapshot'
require 'fn current_session_input_policy\(' "$INPUT" \
  'production input path must resolve current policy per frame'
require 'let snapshot = session\.target_snapshot\(\);' "$INPUT" \
  'production input path must read the latest session target snapshot'
require 'let input_scope = session\.target_binding\(\)\.input_scope\(\);' "$INPUT" \
  'production input path must read input scope from the session-owned target binding'
require 'if !snapshot\.input_enabled\(\)' "$INPUT" \
  'production input path must disable input after target loss'
require 'input_policy_for_scope\(input_policy, input_scope\)' "$INPUT" \
  'production input path must reapply input scope after deriving latest pointer target geometry'
require 'policy\["pointer_target"\]\["target_geometry_revision"\]' "$INPUT" \
  'input policy test must assert pointer target geometry revision'
require 'loose base policy reopen view-only pointer input' "$INPUT" \
  'input policy tests must prove loose base policies cannot reopen view-only pointer input'
require 'loose base policy reopen view-only keyboard input' "$INPUT" \
  'input policy tests must prove loose base policies cannot reopen view-only keyboard input'
require 'observe_bound_session_target_once' "$TARGET_OBSERVER" \
  'target observer must expose the bound-session observation boundary'
require 'record_target_observation_for_session' "$TARGET_OBSERVER" \
  'target observer must commit through the session store boundary'
require 'return TargetObservationPollResult::stop_tracking\(\);' "$TARGET_OBSERVER" \
  'target observer must return stop_tracking when the session is missing or terminal'
require 'observation_provider_commits_through_session_store_boundary' "$TARGET_OBSERVER" \
  'E2E-08 must test observer-to-session-store geometry commits'
require 'observer_stops_tracking_missing_or_terminal_sessions_without_polling_provider' "$TARGET_OBSERVER" \
  'target observer tests must prove missing/terminal sessions stop tracking without polling host state'
require 'stale_observation_cannot_commit_after_session_binding_reuse' "$TARGET_OBSERVER" \
  'stale observations must not advance a reused session binding'
require 'unsupported_platform_target_observation' "$TARGET_OBSERVER" \
  'target observer must centralize unsupported platform app/window fail-closed semantics'
require_multiline 'm/#\[cfg\(not\(target_os = .macos.\)\)\]\s*mod platform.+?unsupported_platform_target_observation\(binding\)/s' "$TARGET_OBSERVER" \
  'non-macOS platform target observer must fail app/window targets closed instead of silently returning no observation'
require 'unsupported_platform_observer_fails_app_window_targets_closed' "$TARGET_OBSERVER" \
  'target observer tests must prove unsupported platforms fail app/window targets closed'
require 'RemoteDesktopTargetMonitor' "$RUNTIME" \
  'remote desktop runtime must own a plugin-scoped target monitor'
require 'fn track_session_target\(' "$RUNTIME" \
  'remote desktop runtime must expose a target tracking registration boundary'
require 'fn schedule_session_lease\(' "$RUNTIME" \
  'remote desktop runtime must expose a fallible lease scheduler registration boundary'
require '-> anyhow::Result<\(\)>' "$RUNTIME" \
  'remote desktop monitor registration boundaries must propagate spawn/send failures'
require 'plugin\.target_monitor\.track\(plugin, session_id\)' "$RUNTIME" \
  'target tracking registration must go through the plugin-owned target monitor'
require_multiline 'm/plugin\s*\.\s*lease_monitor\s*\.\s*schedule\(plugin, session_id, lease_expires_at_ms\)/s' "$RUNTIME" \
  'lease scheduling registration must go through the plugin-owned lease monitor'
require 'fn cancel_session_target_tracking\(' "$RUNTIME" \
  'remote desktop runtime must expose a target tracking cancellation boundary'
require 'self\.target_monitor\.cancel\(session_id\)' "$RUNTIME" \
  'target tracking cancellation must go through the plugin-owned target monitor'
require 'RemoteDesktopPlugin::track_session_target\(&plugin, tracker_session_id' "$CREATE_SESSION" \
  'create_session must register created sessions with the target monitor'
require_multiline 'm/RemoteDesktopPlugin::schedule_session_lease\(\s*&plugin,\s*watchdog_session_id\.clone\(\),\s*lease_expires_at_ms,?\s*\)/s' "$CREATE_SESSION" \
  'create_session must register created sessions with the lease monitor before returning'
require 'remove_inserted_session\(&plugin, &tracker_session_id\)' "$CREATE_SESSION" \
  'create_session must roll back the inserted row when monitor registration fails'
require 'plugin\.cancel_session_lease\(&watchdog_session_id\)' "$CREATE_SESSION" \
  'create_session must cancel the lease monitor if target tracking registration fails'
require 'plugin\.cancel_session_target_tracking\(session_id\)' "$SESSION_LIFECYCLE" \
  'terminal session cleanup must cancel target tracking'
reject 'expect\("spawn remote desktop lease monitor"\)' "$LEASE_MONITOR" \
  'lease monitor worker spawn must propagate errors instead of panicking'
reject 'expect\("spawn remote desktop target monitor"\)' "$TARGET_MONITOR" \
  'target monitor worker spawn must propagate errors instead of panicking'
require 'fn apply_command\(command: TargetMonitorCommand, tracked: &mut HashSet<String>\) -> bool' "$TARGET_MONITOR" \
  'target monitor must centralize command state transitions'
require 'TargetMonitorCommand::Track \{ session_id \}' "$TARGET_MONITOR" \
  'target monitor command state machine must handle Track explicitly'
require 'TargetMonitorCommand::Cancel \{ session_id \}' "$TARGET_MONITOR" \
  'target monitor command state machine must handle Cancel explicitly'
require 'tracked\.remove\(&session_id\)' "$TARGET_MONITOR" \
  'target monitor Cancel command must remove the session id from the tracked set'
require 'TargetMonitorCommand::Shutdown => false' "$TARGET_MONITOR" \
  'target monitor command state machine must handle Shutdown as the terminal command'
require 'target_monitor_command_state_machine_tracks_cancels_and_shuts_down' "$TARGET_MONITOR" \
  'target monitor must test track/cancel/shutdown command semantics'
require 'fn push_target_tracking_event\(' "$SESSION" \
  'session aggregate must own target-event transport epoch projection'
require 'payload\["transport_epoch"\]' "$SESSION" \
  'target lifecycle event payloads must include current transport_epoch before event-log projection'
require 'self\.transport\.active_epoch\(\)' "$SESSION" \
  'target lifecycle event transport_epoch must come from the session transport state'
require 'self\.push_target_tracking_event\(event\)' "$SESSION" \
  'record_target_observation must write target events through the session aggregate projection boundary'
require 'target_tracking_events_include_active_transport_epoch_at_session_boundary' "$SESSION" \
  'E2E-08 must prove target lifecycle events carry the active transport epoch'
require 'input_blocked_reason' "$TARGET_TRACKING" \
  'target snapshot must expose explicit input block reason'
require 'fn input_blocked_reason\(' "$TARGET_TRACKING" \
  'target snapshot must derive a single machine-readable input block reason'
require 'target_loss_pending' "$TARGET_TRACKING" \
  'pending target loss debounce must block input before committed target loss'
require 'tracker_debounces_single_transient_lost_observation' "$TARGET_TRACKING" \
  'target tracker must test pending-loss debounce input safety'
require 'pending_target_loss_deactivates_input_before_media_loss_debounce' "$SESSION" \
  'session aggregate must deactivate input during target-loss debounce before media loss commits'

# E2E-09: target loss is not transport failure. The session aggregate must
# degrade the media source, disable input, and project MEDIA_SOURCE_LOST with
# failure_domain=target after TARGET_LOST.
require 'fn record_target_observation\(' "$SESSION" \
  'session aggregate must be the committed target observation writer'
require 'let target_loss_reason = match &observation' "$SESSION" \
  'session must branch typed target loss reasons separately from transport failure'
require 'TargetObservation::Lost \{ reason, \.\. \} => Some\(\*reason\)' "$SESSION" \
  'session target-loss branch must preserve typed target loss reason'
require 'TargetObservation::PermissionRevoked' "$SESSION" \
  'session target-loss branch must handle permission revocation separately from transport failure'
require 'Suspended,' "$CONTRACT" \
  'remote desktop lifecycle must expose an explicit Suspended state'
require 'REMOTE_DESKTOP_SESSION_STATE_SUSPENDED' "$CONTRACT" \
  'remote desktop lifecycle must expose a canonical Suspended wire state'
require 'fn suspend\(' "$SESSION_STATE" \
  'session state machine must centralize target-loss suspension'
require 'self\.lifecycle\.suspend\(\)' "$SESSION" \
  'target loss must suspend the session lifecycle'
reject_multiline 'm/target_loss_reason\.is_some\(\).{0,240}?mark_degraded\(\)/s' "$SESSION" \
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
require_multiline 'm/media_source_lost\(\s*self\.target\.binding\(\)/s' "$SESSION" \
  'MEDIA_SOURCE_LOST projection must consume the committed session target binding'
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
require_multiline 'm/all\(\|event\| event\["event_type"\] != json!\("SESSION_DEGRADED"\)\)/s' "$SESSION" \
  'target-domain media source loss tests must assert SESSION_DEGRADED is not emitted'
require 'lost_observation_returns_media_source_stop_effect_after_debounce' "$TARGET_OBSERVER" \
  'E2E-09 must test observer-debounced target loss returns media stop effect'
require_multiline 'm/fn observe_window\(.+?owner_matches.+?window\.visibility_state != TargetVisibilityState::Visible.+?snapshot\.title\(\).+?snapshot\.focused\(\)/s' "$TARGET_OBSERVER" \
  'window observer must prioritize hidden/minimized availability before title/focus updates'
require 'window_observation_prioritizes_visibility_loss_over_title_or_focus_changes' "$TARGET_OBSERVER" \
  'target observer tests must prove hidden/minimized availability outranks title/focus updates'
require 'MEDIA_SOURCE_LOST' "$SESSION_EVENTS" \
  'session events must project MEDIA_SOURCE_LOST'
require 'struct RemoteDesktopEventProjection' "$SESSION_EVENTS" \
  'session event projection must be a domain object, not a tuple alias'
require "fn new\\(event_type: &'static str, payload: Value\\) -> Self" "$SESSION_EVENTS" \
  'session event projection must centralize event_type/payload construction'
require "fn event_type\\(&self\\) -> &'static str" "$SESSION_EVENTS" \
  'session aggregate must read event_type through the projection domain object'
require 'fn into_payload\(self\) -> Value' "$SESSION_EVENTS" \
  'session aggregate must consume event payload through the projection domain object'
reject "type RemoteDesktopEventProjection = \\(&'static str, Value\\)" "$SESSION_EVENTS" \
  'session event projection must not regress to a tuple alias'
require 'fn push_projected_event\(&mut self, event: session_events::RemoteDesktopEventProjection\)' "$SESSION" \
  'session aggregate must only accept typed remote desktop event projections'
reject "fn push_projected_event\\(&mut self, event: \\(&'static str, Value\\)\\)" "$SESSION" \
  'session aggregate must not accept arbitrary event_type/payload tuples'
require 'binding: &RemoteAppTargetBinding' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST event builder must require committed target binding context'
require '"subject_ura": binding\.subject_ura\(\)' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry subject URA'
require '"binding_id": binding\.binding_id\(\)' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry binding id'
require_multiline '/fn media_source_lost\((?:(?!fn webrtc_transport_failure_context).)*"binding_id": binding\.binding_id\(\)/s' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry binding id from the committed binding'
require '"binding_epoch": binding\.binding_epoch\(\)' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry binding epoch'
require '"target_identity_epoch": binding\.target_identity_epoch\(\)' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry target identity epoch'
require '"target_geometry_revision": binding\.target_geometry_revision\(\)' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry target geometry revision'
require '"media_source_epoch": binding\.media_source_epoch\(\)' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry media source epoch'
require_multiline '/fn media_source_lost\((?:(?!fn client_media_reason_code).)*"consent_epoch": binding\.consent_epoch\(\)/s' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry consent epoch'
require '"failure_domain": "target"' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST must be a target-domain failure'
require '"media_transport_ready": false' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST must mark media transport unavailable'
require 'fn session_degraded\(' "$SESSION_EVENTS" \
  'session events must project explicit SESSION_DEGRADED lifecycle rows'
require '"SESSION_DEGRADED"' "$SESSION_EVENTS" \
  'SESSION_DEGRADED event type must be emitted by the session event builder'
require 'fn client_media_reason_code\(' "$SESSION_EVENTS" \
  'client media degradation reason_code derivation must be centralized'
require '"client_media_stalled"' "$SESSION_EVENTS" \
  'client media stalled degradation must have a typed reason_code'
require_multiline '/fn session_degraded\((?:(?!fn client_media_reason_code).)*"recoverability": "retry_session"/s' "$SESSION_EVENTS" \
  'SESSION_DEGRADED must expose a retry recovery class'
require '"failure_domain": "client_media"' "$SESSION_EVENTS" \
  'SESSION_DEGRADED must stay separate from target and transport failure domains'
require '"frontend_action": FrontendAction::RetrySession\.as_str\(\)' "$SESSION_EVENTS" \
  'SESSION_DEGRADED must carry an actionable frontend recovery hint'
require '"primary_phase": primary_phase' "$SESSION_EVENTS" \
  'SESSION_DEGRADED must expose the transport primary phase that caused degradation'
require 'session_degraded_payload_projects_recovery_context' "$SESSION_EVENTS" \
  'session event tests must prove SESSION_DEGRADED recovery payload shape'
require 'session_events::session_degraded' "$SESSION" \
  'session aggregate must emit SESSION_DEGRADED from lifecycle health projection'
require 'client_media_stall_emits_session_degraded_recovery_event' "$SESSION" \
  'session aggregate tests must prove client media stall emits SESSION_DEGRADED'
require 'client_state_index < degraded_index' "$SESSION" \
  'client media stall test must prove the cause event precedes SESSION_DEGRADED'
require 'degraded\["reason_code"\], json!\("client_media_stalled"\)' "$SESSION" \
  'SESSION_DEGRADED aggregate test must assert top-level reason_code'
require 'degraded\["recoverability"\], json!\("retry_session"\)' "$SESSION" \
  'SESSION_DEGRADED aggregate test must assert top-level recoverability'
require 'degraded\["payload"\]\["primary_phase"\], json!\("degraded"\)' "$SESSION" \
  'SESSION_DEGRADED aggregate test must assert degraded transport phase'
require 'enum WebRtcFailureEventKind' "$SESSION_EVENTS" \
  'WebRTC failure projection must use explicit SPEC event taxonomy'
require 'Self::MediaSourceLost => "MEDIA_SOURCE_LOST"' "$SESSION_EVENTS" \
  'WebRTC target/media-source failures must project MEDIA_SOURCE_LOST'
require 'Self::TransportFailed => "TRANSPORT_FAILED"' "$SESSION_EVENTS" \
  'WebRTC transport failures must project TRANSPORT_FAILED'
reject '"SESSION_FAILED"' "$SESSION_EVENTS" \
  'remote desktop event stream must not collapse typed WebRTC failures into SESSION_FAILED'
require 'WebRtcFailureEventKind::MediaSourceLost' "$WEBRTC_MEDIA" \
  'native target failures must be projected as media-source loss, not generic session failure'
require 'WebRtcFailureEventKind::TransportFailed' "$SESSION_STORE" \
  'direct WebRTC endpoint failures must default to TRANSPORT_FAILED'
require 'fn webrtc_transport_failure_context' "$SESSION_EVENTS" \
  'direct WebRTC transport failures must share one canonical recovery context helper'
require '"reason_code": TargetResolutionError::TransportRouteUnavailable\.as_str\(\)' "$SESSION_EVENTS" \
  'direct WebRTC transport failure context must publish canonical transport_route_unavailable reason_code'
require_multiline '/fn webrtc_transport_failure_context\((?:(?!fn session_created).)*"recoverability": "retry_session"/s' "$SESSION_EVENTS" \
  'direct WebRTC transport failure context must publish retry_session recoverability'
require '"failure_domain": "transport"' "$SESSION_EVENTS" \
  'direct WebRTC transport failure context must identify the transport domain'
require_multiline '/fn webrtc_transport_failure_context\((?:(?!fn session_created).)*FrontendAction::RetrySession\.as_str\(\)/s' "$SESSION_EVENTS" \
  'direct WebRTC transport failure context must publish retry_session recovery action'
require 'webrtc_transport_failure_context\(\)' "$SESSION_STORE" \
  'direct WebRTC default failure path must not emit empty transport failure context'
require 'direct_webrtc_transport_failure_projects_recovery_context' "$SESSION_STORE" \
  'session-store tests must prove default transport failures publish recovery context'
require 'event\["reason_code"\]' "$SESSION_STORE" \
  'session-store tests must prove TRANSPORT_FAILED top-level reason_code is projected'
require 'event\["recoverability"\]' "$SESSION_STORE" \
  'session-store tests must prove TRANSPORT_FAILED top-level recoverability is projected'
require_multiline '/fn session_created\((?:(?!fn capture_target_resolved).)*"reason_code": "session_created",(?:(?!fn capture_target_resolved).)*"recoverability": "continue"/s' "$SESSION_EVENTS" \
  'SESSION_CREATED path must publish initial reason_code and continue recoverability'
require_multiline '/fn capture_target_resolved\((?:(?!fn session_closed)(?!fn target_bound).)*"subject_ura": binding\.subject_ura\(\),(?:(?!fn session_closed)(?!fn target_bound).)*"binding_id": binding\.binding_id\(\),(?:(?!fn session_closed)(?!fn target_bound).)*"binding_epoch": binding\.binding_epoch\(\),(?:(?!fn session_closed)(?!fn target_bound).)*"previous_target_identity_epoch": Value::Null,(?:(?!fn session_closed)(?!fn target_bound).)*"target_identity_epoch": binding\.target_identity_epoch\(\),(?:(?!fn session_closed)(?!fn target_bound).)*"target_geometry_revision": binding\.target_geometry_revision\(\),(?:(?!fn session_closed)(?!fn target_bound).)*"media_source_epoch": binding\.media_source_epoch\(\),(?:(?!fn session_closed)(?!fn target_bound).)*"consent_epoch": binding\.consent_epoch\(\),(?:(?!fn session_closed)(?!fn target_bound).)*"reason_code": "capture_target_resolved",(?:(?!fn session_closed)(?!fn target_bound).)*"recoverability": "continue"/s' "$SESSION_EVENTS" \
  'CAPTURE_TARGET_RESOLVED path must publish initial binding context and continue recoverability'
require_multiline '/fn session_closing\((?:(?!fn session_closed).)*"reason_code": reason,(?:(?!fn session_closed).)*"recoverability": "closing"/s' "$SESSION_EVENTS" \
  'SESSION_CLOSING path must publish terminal reason_code and closing recoverability'
require_multiline '/fn session_closed\((?:(?!fn session_expired).)*"reason_code": reason,(?:(?!fn session_expired).)*"recoverability": "closed"/s' "$SESSION_EVENTS" \
  'SESSION_CLOSED caller path must publish terminal reason_code and closed recoverability'
require_multiline '/fn session_expired\((?:(?!fn webrtc_sender_ready).)*"reason_code": reason,(?:(?!fn webrtc_sender_ready).)*"recoverability": "closed"/s' "$SESSION_EVENTS" \
  'SESSION_CLOSED lease-expiry path must publish terminal reason_code and closed recoverability'
require 'session_closing_payload_projects_terminal_reason_code' "$SESSION_EVENTS" \
  'session event tests must prove closing payload publishes terminal reason_code'
require 'session_created_payload_projects_initial_reason_code' "$SESSION_EVENTS" \
  'session event tests must prove SESSION_CREATED publishes initial reason_code'
require 'capture_target_resolved_payload_projects_initial_binding_context' "$SESSION_EVENTS" \
  'session event tests must prove CAPTURE_TARGET_RESOLVED publishes binding context'
require 'payload\["consent_epoch"\]' "$SESSION_EVENTS" \
  'session event tests must prove target binding events publish consent epoch'
require 'session_closed_payload_projects_terminal_reason_code' "$SESSION_EVENTS" \
  'session event tests must prove caller close payload publishes terminal reason_code'
require 'session_expired_payload_projects_terminal_reason_code' "$SESSION_EVENTS" \
  'session event tests must prove lease expiry payload publishes terminal reason_code'
require 'initial_session_events_project_reason_codes_in_order' "$SESSION" \
  'session aggregate tests must prove initial event-log top-level reason_code order'
require 'created_index < resolved_index && resolved_index < bound_index' "$SESSION" \
  'session aggregate tests must prove initial session, resolution, and bound events are ordered'
require 'resolved\["binding_id"\]' "$SESSION" \
  'session aggregate tests must prove CAPTURE_TARGET_RESOLVED top-level binding id is projected'
require 'session_close_events_project_terminal_reason_code' "$SESSION" \
  'session aggregate tests must prove close event-log top-level reason_code is projected'
require 'closing_index < closed_index' "$SESSION" \
  'session aggregate tests must prove SESSION_CLOSING precedes terminal SESSION_CLOSED'
require 'closing\["recoverability"\]' "$SESSION" \
  'session aggregate tests must prove SESSION_CLOSING top-level recoverability is projected'
require 'session_expiry_events_project_terminal_reason_code' "$SESSION" \
  'session aggregate tests must prove expiry event-log top-level reason_code is projected'
require 'events\[media_source_lost_index\]\["binding_id"\]' "$SESSION" \
  'E2E-09 must assert event-log top-level MEDIA_SOURCE_LOST binding id'
require 'events\[media_source_lost_index\]\["target_identity_epoch"\]' "$SESSION" \
  'E2E-09 must assert event-log top-level MEDIA_SOURCE_LOST target identity epoch'
require 'events\[media_source_lost_index\]\["media_source_epoch"\]' "$SESSION" \
  'E2E-09 must assert event-log top-level MEDIA_SOURCE_LOST media source epoch'
require 'events\[media_source_lost_index\]\["consent_epoch"\]' "$SESSION" \
  'E2E-09 must assert event-log top-level MEDIA_SOURCE_LOST consent epoch'
reject 'TRANSPORT_FAILED' "$SESSION" \
  'session target-loss tests must not rely on a transport-failed event'
require 'target_failure_payload' "$TARGET_TRACKING" \
  'target failure events must share one projection for failure-domain recovery fields'
require 'payload\["failure_domain"\] = json!\("target"\)' "$TARGET_TRACKING" \
  'TARGET_LOST and permission revocation payloads must be target-domain failures'
require 'events\[target_lost_index\]\["payload"\]\["frontend_action"\]' "$SESSION" \
  'E2E-09 must assert TARGET_LOST carries frontend recovery action'
require 'events\[target_lost_index\]\["payload"\]\["failure_domain"\]' "$SESSION" \
  'E2E-09 must assert TARGET_LOST is a target-domain failure'
require 'display_topology_loss_projects_target_failure_recovery' "$TARGET_TRACKING" \
  'display topology loss must have target-domain failure recovery coverage'
require_multiline 'm/TargetResolutionError::TargetDisplayUnavailable\s*\.frontend_action\(\)/s' "$TARGET_TRACKING" \
  'selected display loss must use canonical target-display-unavailable frontend action'
require 'json!\("target_display_unavailable"\)' "$TARGET_TRACKING" \
  'display topology loss test must assert target_display_unavailable reason code'
require 'topology_changed\.payload\(\)\["input_blocked_reason"\]' "$TARGET_TRACKING" \
  'display topology loss test must assert input_blocked_reason for frontend recovery'
require 'TargetResolutionError::TargetHidden' "$TARGET_TRACKING" \
  'hidden target visibility must use canonical target_hidden reason'
require 'TargetResolutionError::TargetMinimized' "$TARGET_TRACKING" \
  'minimized target visibility must use canonical target_minimized reason'
require 'json!\("target_hidden"\)' "$TARGET_TRACKING" \
  'hidden visibility test must assert target_hidden reason code'
require 'json!\("target_minimized"\)' "$TARGET_TRACKING" \
  'minimized visibility test must assert target_minimized reason code'
require 'hidden\.payload\(\)\["input_blocked_reason"\]' "$TARGET_TRACKING" \
  'hidden visibility test must assert input_blocked_reason for frontend recovery'
require 'minimized\.payload\(\)\["input_blocked_reason"\]' "$TARGET_TRACKING" \
  'minimized visibility test must assert input_blocked_reason for frontend recovery'
require 'json!\("retry_session"\)' "$TARGET_TRACKING" \
  'hidden/minimized visibility tests must assert canonical retry_session action'
require 'FrontendAction::RetrySession' "$TARGET_TRACKING" \
  'focus-loss target visibility must use canonical retry_session action'
require 'TARGET_BLURRED' "$TARGET_TRACKING" \
  'focus loss must emit TARGET_BLURRED'
require 'json!\("target_blurred"\)' "$TARGET_TRACKING" \
  'focus loss test must assert target_blurred reason code'
require 'blurred\.payload\(\)\["input_blocked_reason"\]' "$TARGET_TRACKING" \
  'focus loss test must assert input_blocked_reason for frontend recovery'
require '"TARGET_REBIND_FAILED"' "$TARGET_TRACKING" \
  'post-loss target observations must emit an explicit rebind failure when no Rebinding policy exists'
require 'explicit_rebind_required' "$TARGET_TRACKING" \
  'TARGET_REBIND_FAILED must expose explicit_rebind_required'
require 'target_status' "$TARGET_TRACKING" \
  'TARGET_REBIND_FAILED must expose lost target status so frontend cannot treat it as a normal geometry update'
require '"input_enabled": false' "$TARGET_TRACKING" \
  'TARGET_REBIND_FAILED must keep target-scoped input disabled'
require 'rebind_attempted\.payload\(\)\["failure_domain"\]' "$TARGET_TRACKING" \
  'TARGET_REBIND_ATTEMPTED must project target failure domain'
require 'rebind_failed\.payload\(\)\["failure_domain"\]' "$TARGET_TRACKING" \
  'TARGET_REBIND_FAILED must project target failure domain'
require 'tracker\.snapshot\(\)\.latest_diagnostic\(\)\["failure_domain"\]' "$TARGET_TRACKING" \
  'rebind latest diagnostics must project target failure domain'
require 'tracker_reports_rebind_failure_after_target_loss_without_policy' "$TARGET_TRACKING" \
  'target tracker must test explicit rebind failure instead of silently swallowing post-loss observations'
require 'target_title_after_loss' "$TARGET_TRACKING" \
  'post-loss title observations must enter explicit rebind instead of being silently swallowed'
require 'target_focus_after_loss' "$TARGET_TRACKING" \
  'post-loss focus observations must enter explicit rebind instead of being silently swallowed'
require 'tracker_routes_post_loss_title_focus_through_explicit_rebind' "$TARGET_TRACKING" \
  'target tracker must test title/focus reappearance through explicit rebind semantics'
require 'fn window_set_epoch\(' "$TARGET" \
  'application window-set proof must expose the recomputed identity epoch'
require 'ApplicationWindowSetChanged' "$TARGET_TRACKING" \
  'target tracking must model same-app application window-set changes explicitly'
require 'update_application_window_set' "$TARGET" \
  'session-owned target binding must update application window-set state through a domain method'
require 'AppWindowSetProof::new' "$TARGET_OBSERVER" \
  'application observer must rederive the current display-scoped app window-set proof'
require 'TargetObservation::ApplicationWindowSetChanged' "$TARGET_OBSERVER" \
  'application observer must report same-app window-set drift as a rebind observation'
require 'application_observer_reports_committed_window_set_drift_as_rebind' "$TARGET_OBSERVER" \
  'target observer must test committed application window-set expansion/contraction rebind evidence'
require 'application_observation_rebinds_same_display_window_set_expansion' "$TARGET_OBSERVER" \
  'application observer must test same-display app window-set expansion rebind evidence'
require 'application_observation_rebinds_same_app_window_set_subset' "$TARGET_OBSERVER" \
  'application observer must test same-display app window-set contraction rebind evidence'
require 'snapshot_observer_reappearance_requires_explicit_rebind_policy' "$TARGET_OBSERVER" \
  'target observer must prove platform-visible target reappearance cannot revive media/input without explicit rebind policy'
require 'target_reappearance_after_loss_emits_explicit_rebind_failure' "$SESSION" \
  'session aggregate must test post-loss target reappearance as TARGET_REBIND_FAILED'
require_multiline 'm/rebind_attempted\["reason_code"\]\s*,\s*json!\("target_rebind_attempted"\)/s' "$SESSION" \
  'session aggregate must assert TARGET_REBIND_ATTEMPTED top-level reason_code'
require_multiline 'm/rebind_attempted\["recoverability"\]\s*,\s*json!\("retry_session"\)/s' "$SESSION" \
  'session aggregate must assert TARGET_REBIND_ATTEMPTED top-level recoverability'
require 'rebind_attempted\["binding_id"\]' "$SESSION" \
  'session aggregate must assert TARGET_REBIND_ATTEMPTED top-level binding id'
require 'rebind_attempted\["target_identity_epoch"\]' "$SESSION" \
  'session aggregate must assert TARGET_REBIND_ATTEMPTED top-level target identity epoch'
require 'rebind_attempted\["media_source_epoch"\]' "$SESSION" \
  'session aggregate must assert TARGET_REBIND_ATTEMPTED top-level media source epoch'
require_multiline 'm/rebind_failed\["reason_code"\]\s*,\s*json!\("explicit_rebind_required"\)/s' "$SESSION" \
  'session aggregate must assert TARGET_REBIND_FAILED top-level reason_code'
require_multiline 'm/rebind_failed\["recoverability"\]\s*,\s*json!\("new_session_required"\)/s' "$SESSION" \
  'session aggregate must assert TARGET_REBIND_FAILED top-level recoverability'
require 'rebind_failed\["binding_id"\]' "$SESSION" \
  'session aggregate must assert TARGET_REBIND_FAILED top-level binding id'
require 'rebind_failed\["target_identity_epoch"\]' "$SESSION" \
  'session aggregate must assert TARGET_REBIND_FAILED top-level target identity epoch'
require 'rebind_failed\["media_source_epoch"\]' "$SESSION" \
  'session aggregate must assert TARGET_REBIND_FAILED top-level media source epoch'
require 'const TARGET_CHANGED_EVENT_TYPES' "$EVENT_LOG" \
  'event log must centralize SPEC target lifecycle taxonomy instead of scattering target-change strings in match arms'
require 'TARGET_CHANGED_EVENT_TYPES\.contains\(&event_type\)' "$EVENT_LOG" \
  'event log proto projection must consume the centralized target lifecycle taxonomy'
require 'spec_target_lifecycle_events_have_explicit_proto_projection' "$EVENT_LOG" \
  'event log tests must prove every SPEC target lifecycle event has an explicit proto projection'
require '"TARGET_REBIND_FAILED"' "$EVENT_LOG" \
  'event log target lifecycle taxonomy must include TARGET_REBIND_FAILED'
require 'transport_route_state' "$VIEW_TRANSPORT" \
  'transport view must expose host/STUN/TURN/EasyNet relay route state'
require '"host_candidate"' "$VIEW_TRANSPORT" \
  'transport route state must expose host candidates explicitly'
require '"stun_srflx"' "$VIEW_TRANSPORT" \
  'transport route state must expose STUN server-reflexive candidates explicitly'
require '"turn_relay"' "$VIEW_TRANSPORT" \
  'transport route state must expose TURN relay candidates explicitly'
require '"easynet_relay"' "$VIEW_TRANSPORT" \
  'transport route state must expose EasyNet relay readiness explicitly'
require '"failed"' "$VIEW_TRANSPORT" \
  'transport route state must expose failed route state explicitly'
require 'route_state' "$VIEW" \
  'public session view must project route state'
require 'transport_route_state\.clone\(\)' "$VIEW" \
  'public session view must reuse one route-state projection across signaling/readiness'
require 'fn production_scope_ready\(' "$TARGET" \
  'target binding must own the production scope readiness predicate'
require 'self\.target\.binding\(\)\.production_scope_ready\(\)' "$SESSION" \
  'production media readiness must be gated by target binding scope readiness'
require '&& self\.transport\.client_media_ready\(\)' "$SESSION" \
  'production media readiness must wait for client presenting evidence, not only device sender readiness'
require '"target_scope_ready": session\.target_scope_ready\(\)' "$VIEW" \
  'public production readiness must expose target scope readiness'
require '"blocked_reason": production_readiness_blocked_reason\(session\)' "$VIEW" \
  'public production readiness must expose one typed blocked_reason instead of forcing UI inference'
require 'fn production_readiness_blocked_reason\(' "$VIEW" \
  'production readiness blocked reason must be centralized in the session view projection'
require '"target_scope_not_ready"' "$VIEW" \
  'production readiness must distinguish target scope/fallback blockers'
require '"production_codec_not_negotiated"' "$VIEW" \
  'production readiness must distinguish non-production or missing codec blockers'
require '"media_transport_not_ready"' "$VIEW" \
  'production readiness must distinguish media transport blockers'
require '"client_media_not_presenting"' "$VIEW" \
  'production readiness must distinguish missing client presenting/decoded evidence'
require 'production_media_ready_requires_target_scope_ready' "$SESSION" \
  'production online predicate must test scope fallback/widening rejection'
require 'scope widening or display fallback must prevent production online' "$SESSION" \
  'production readiness test must assert fallback/widening cannot report online'
require 'production_media_ready_requires_production_codec_and_sender_ready' "$SESSION_STORE" \
  'production readiness must test non-production codec and sender readiness blockers'
require 'view\["production_readiness"\]\["blocked_reason"\]' "$SESSION_STORE" \
  'production readiness tests must assert the public blocked_reason projection'
require 'view\["production_readiness"\]\["client_media_ready"\]' "$SESSION_STORE" \
  'production readiness tests must assert client presenting evidence before online'
require 'report_client_media_state\(TransportEpoch::new\(1\), "presenting"\)' "$SESSION_STORE" \
  'production readiness tests must prove client presenting flips production online after sender readiness'
require 'view\["transport"\]\["production_ready"\]' "$SESSION_STORE" \
  'public transport summary must expose production_ready separately from primary_ready'
require 'view\["transports"\]\[0\]\["metadata"\]\["production_ready"\]' "$SESSION_STORE" \
  'public transport list must preserve production_ready evidence in primary transport metadata'
require '"production_ready": self\.production_ready\(session\)' "$VIEW_TRANSPORT" \
  'transport projection must expose route-gated production_ready separately from primary_ready'
require '"scope_widened": self\.scope_audit\.scope_widened' "$TARGET" \
  'TARGET_BOUND payload must project scope widening from the committed binding audit'
require '"display_fallback_used": self\.scope_audit\.display_fallback_used' "$TARGET" \
  'TARGET_BOUND payload must project display fallback from the committed binding audit'
require '"consent_epoch": self\.consent_epoch' "$TARGET" \
  'TARGET_BOUND payload must project consent epoch from the committed binding'
require 'target_bound\["payload"\]\["display_fallback_used"\]' "$SESSION" \
  'production readiness test must assert TARGET_BOUND fallback projection'
require 'bound\["consent_epoch"\]|target_bound\["consent_epoch"\]' "$SESSION" \
  'session tests must assert TARGET_BOUND top-level consent epoch projection'
require 'TargetResolutionError::TransportRouteUnavailable' "$VIEW_TRANSPORT" \
  'transport route degradation must expose the SPEC canonical transport_route_unavailable reason code from the shared taxonomy'
require_multiline '/fn summary\([\s\S]*?"message": self\.message,\s*"reason_code": self\.reason_code\.clone\(\)/s' "$VIEW_TRANSPORT" \
  'public transport summary must project canonical route degradation reason_code'
require_multiline '/"metadata": \{[\s\S]*?"input_channel_label": INPUT_DATA_CHANNEL_LABEL,\s*"reason_code": self\.reason_code\.clone\(\)/s' "$VIEW_TRANSPORT" \
  'primary transport metadata must preserve canonical route degradation reason_code'
require 'fn transport_readiness_blocker\(' "$VIEW_TRANSPORT" \
  'transport route reason_code derivation must be centralized in the transport readiness blocker projection'
require 'struct RemoteDesktopTransportReadinessBlocker' "$VIEW_TRANSPORT" \
  'transport route recovery metadata must be centralized in the transport readiness blocker projection'
require 'fn transport_route_failed\(' "$VIEW_TRANSPORT" \
  'transport route failed predicate must be explicit instead of treating every WebRTC error as a route failure'
require 'struct RemoteDesktopTransportBlocker' "$TRANSPORT_BLOCKER" \
  'non-route WebRTC blockers must use one canonical transport blocker taxonomy helper'
require 'from_webrtc_error' "$TRANSPORT_BLOCKER" \
  'transport blocker helper must classify deterministic WebRTC blocker reasons'
require 'TargetResolutionError::CaptureBackendUnavailable' "$TRANSPORT_BLOCKER" \
  'backend-unavailable WebRTC blockers must map to the SPEC capture_backend_unavailable reason code'
require 'TargetResolutionError::ScreenCaptureKitStreamStartFailed' "$TRANSPORT_BLOCKER" \
  'native media pipeline failures must map to the SPEC ScreenCaptureKit stream-start reason code'
require 'RemoteDesktopTransportBlocker::from_webrtc_error' "$VIEW_TRANSPORT" \
  'transport readiness reason_code must reuse the shared non-route blocker taxonomy helper'
require 'RemoteDesktopTransportBlocker::from_webrtc_error' "$SESSION_EVENTS" \
  'TRANSPORT_BLOCKED events must reuse the shared non-route blocker taxonomy helper'
require '"reason_code": blocker\.map\(RemoteDesktopTransportBlocker::reason_code_str\)' "$SESSION_EVENTS" \
  'TRANSPORT_BLOCKED events must project canonical blocker reason_code'
require '"frontend_action": blocker' "$SESSION_EVENTS" \
  'TRANSPORT_BLOCKED events must project frontend recovery action'
require 'transport_blocked_projects_capture_backend_reason_code' "$SESSION_EVENTS" \
  'session event tests must prove TRANSPORT_BLOCKED publishes capture_backend_unavailable'
require 'backend_unavailable_maps_to_capture_backend_unavailable' "$TRANSPORT_BLOCKER" \
  'transport blocker tests must prove backend-unavailable canonical mapping'
reject_multiline '/fn mark_backend_unavailable\((?:(?!fn commit_started_endpoint).)*session\.set_description\(/s' "$WEBRTC_NEGOTIATION" \
  'transport backend-unavailable gate must not partially commit remote SDP signaling'
require 'remote_offer_backend_gate_blocks_without_committing_signaling' "$SET_DESCRIPTION" \
  'set_description tests must prove backend-unavailable gate leaves signaling uncommitted'
require 'remote_description"\], Value::Null' "$SET_DESCRIPTION" \
  'backend-unavailable regression must assert remote_description remains empty'
require 'event\["event_type"\] != json!\("DESCRIPTION_SET"\)' "$SET_DESCRIPTION" \
  'backend-unavailable regression must assert DESCRIPTION_SET is not emitted'
require '"host_only_no_nat_or_relay"' "$VIEW_TRANSPORT" \
  'host-only transport degradation must have a typed unavailable reason'
require '"relay_unavailable"' "$VIEW_TRANSPORT" \
  'relay-unavailable transport degradation must have a typed unavailable reason'
require 'production_route_ready' "$VIEW_TRANSPORT" \
  'transport view must publish production route readiness separately from primary media readiness'
require 'host_only_route_keeps_production_offline_after_client_media_presents' "$VIEW_TRANSPORT" \
  'transport tests must prove host-only routes cannot report production online after client media presents'
require '"production_route_ready": transport_view\.production_route_ready\(\)' "$VIEW" \
  'public production readiness must expose production route readiness'
require '"route_readiness_blocker": transport_view\.readiness_blocker\(\)' "$VIEW" \
  'public production readiness must carry route degradation separately from the media online predicate'
require 'summary\["reason_code"\]' "$VIEW_TRANSPORT" \
  'transport tests must assert canonical route degradation reason_code'
require 'transport_route_unavailable' "$VIEW_TRANSPORT" \
  'transport tests must cover the canonical transport_route_unavailable reason code'
require 'host_only_candidates_are_not_reported_as_nat_or_relay_ready' "$VIEW_TRANSPORT" \
  'transport tests must prove host-only candidates are not NAT/relay readiness'
require 'easynet_relay_does_not_imply_turn_relay' "$VIEW_TRANSPORT" \
  'transport tests must prove EasyNet relay readiness is distinct from TURN relay readiness'
require 'candidate_declares_easynet_relay' "$VIEW_TRANSPORT" \
  'EasyNet relay classification must require explicit relay metadata'
reject 'candidate_text\.contains\("easynet"\)' "$VIEW_TRANSPORT" \
  'EasyNet relay classification must not infer relay type from candidate hostname text'
require 'turn_relay_hostname_containing_easynet_is_not_easynet_relay' "$VIEW_TRANSPORT" \
  'transport tests must prove TURN hostnames containing easynet are not EasyNet relay routes'
require 'srflx_without_relay_reports_typed_relay_unavailable_reason' "$VIEW_TRANSPORT" \
  'transport tests must prove STUN-only candidates expose relay-unavailable degradation'
require 'relay_ready' "$SPEC" \
  'SPEC must name relay_ready as the aggregate any-relay state instead of overloading TURN relay'

# Public transport evidence must remain in the EasyNet URA model. WebRTC is a
# transport kind/carrier, not a routable scheme, and tests must not preserve
# fake endpoint schemes that would teach callers to treat endpoint_ura as a raw
# transport address.
require 'fn direct_webrtc_endpoint_ura\(session_id: &str\) -> String' "$CONSTANTS" \
  'direct WebRTC endpoint evidence must be generated through one canonical URA helper'
require 'easynet:///r/local/resource/remote-desktop-transport\.' "$CONSTANTS" \
  'direct WebRTC endpoint helper must publish a transport endpoint resource URA'
require '/endpoint/webrtc' "$CONSTANTS" \
  'direct WebRTC endpoint helper must identify a WebRTC endpoint resource, not a session subject'
require 'hex::encode\(session_id\.as_bytes\(\)\)' "$CONSTANTS" \
  'direct WebRTC endpoint helper must encode raw session ids before inserting them into URA path segments'
require 'direct_webrtc_endpoint_ura\(session_id\)' "$SESSION_STORE" \
  'session store media-ready projection must use the canonical direct WebRTC endpoint URA helper'
require 'direct_webrtc_endpoint_ura\(&endpoint_config\.session_id\)' "$WEBRTC_ENDPOINT" \
  'direct WebRTC answer payload must expose canonical endpoint_ura evidence'
require 'direct_webrtc_endpoint_ura\(session_id\)' "$WEBRTC_NEGOTIATION" \
  'negotiation commit must persist the same canonical endpoint_ura evidence as the answer payload'
require 'direct_webrtc_endpoint_ura\(session\.session_id\(\)\)' "$VIEW_TRANSPORT" \
  'public transport view must derive endpoint_ura from the canonical direct WebRTC endpoint helper'
reject 'webrtc://direct/|easynet-rd://|ura://endpoint' "$REMOTE_ROOT" \
  'remote desktop endpoint_ura evidence must be EasyNet URA only'
reject '"endpoint_ura": "ability:' "$REMOTE_ROOT" \
  'diagnostic ability names must not be projected through endpoint_ura'
reject 'screen\.subscribe' "$REMOTE_ROOT" \
  'remote desktop diagnostic preview must not project the unrelated screen.subscribe ability'
require 'preview_ability": ABILITY_ATTACH_SESSION' "$VIEW_TRANSPORT" \
  'transport summary must project remote_desktop.attach as the diagnostic preview ability'
require 'preview_ability": ABILITY_ATTACH_SESSION' "$SESSION_EVENTS" \
  'session-created event must project remote_desktop.attach as the diagnostic preview ability'
require 'transport_summary_projects_remote_desktop_attach_as_preview_ability' "$VIEW_TRANSPORT" \
  'transport tests must prove preview_ability uses the remote desktop attach ability'
require 'session_created_projects_remote_desktop_attach_as_preview_ability' "$SESSION_EVENTS" \
  'event tests must prove preview_ability uses the remote desktop attach ability'

# Consent is a session aggregate sub-state, not an immutable profile field. The
# current public grant remains visible for audit, but input/media gates must read
# active consent state from the aggregate.
require 'enum RemoteDesktopConsentPhase' "$SESSION_CONSENT_STATE" \
  'remote desktop session must have an explicit consent state machine'
require 'struct RemoteDesktopConsentState' "$SESSION_CONSENT_STATE" \
  'session aggregate must own consent lifecycle state'
require 'RemoteDesktopConsentPhase::Active' "$SESSION_CONSENT_STATE" \
  'inserted sessions must represent active consent explicitly'
require 'RemoteDesktopConsentPhase::Revoked' "$SESSION_CONSENT_STATE" \
  'consent state machine must represent revoked consent'
require 'RemoteDesktopConsentPhase::Expired' "$SESSION_CONSENT_STATE" \
  'consent state machine must represent expired consent'
require 'fn permits_media_input\(' "$SESSION_CONSENT_STATE" \
  'consent state must expose the media/input gate predicate'
require 'consent: RemoteDesktopConsentState' "$SESSION" \
  'session aggregate must own dynamic consent state'
require 'RemoteDesktopConsentState::active' "$SESSION" \
  'session aggregate must activate consent only after creation workflow succeeds'
reject_multiline 'm/struct RemoteDesktopSessionProfile\s*\{[^}]*consent:\s*RemoteDesktopConsentGrant/s' "$SESSION_IDENTITY" \
  'immutable session profile must not own dynamic consent state'
require 'session\.consent_state\(\)\.to_value\(\)' "$VIEW" \
  'public session view must project aggregate consent state'
require 'if !self\.consent\.permits_media_input\(\)' "$SESSION" \
  'input activation must require active consent'
require 'TargetObservation::PermissionRevoked' "$SESSION" \
  'session aggregate must consume permission revocation observations'
require 'self\.consent\.revoke\(\)' "$SESSION" \
  'consent revocation must advance the consent state machine'
require '"TARGET_PERMISSION_REVOKED"' "$TARGET_TRACKING" \
  'permission revocation must project TARGET_PERMISSION_REVOKED through target tracking'
require 'self\.transport\.mark_media_source_lost\(epoch\)' "$SESSION" \
  'consent revocation must mark active media source lost'
require 'self\.consent\.expire\(\)' "$SESSION" \
  'terminal session lifecycle must expire active consent'
require 'consent_revocation_suspends_media_and_blocks_input_activation' "$SESSION" \
  'consent revocation must have session-level media/input regression coverage'
require 'permission_revoked_index < media_source_lost_index' "$SESSION" \
  'consent revocation test must prove permission event precedes media source loss'
require 'revoked consent must prevent input from reactivating' "$SESSION" \
  'consent revocation test must prove inactive consent blocks input activation'

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

# E2E-11: capture/session consent is not input consent. Display sessions and
# app/window sessions remain view-only until explicit input consent exists; app
# and window sessions additionally require a focus-safe target-local input
# validator. Pointer geometry may be computed for diagnostics, but
# keyboard/pointer injection is disabled without those proofs.
require 'fn input_scope_for_request\(' "$TARGET" \
  'target resolver must centralize requested mode to input scope'
require 'InputScopeDecision' "$TARGET" \
  'target resolver must return an explicit input scope decision, not a bare scope'
require 'RemoteDesktopTargetKind::Window \| RemoteDesktopTargetKind::Application' "$TARGET" \
  'window/application targets must share the view-only input safety branch'
require 'InputScope::ViewOnly' "$TARGET" \
  'requested interactive mode must downgrade to view_only when input authority is missing'
require 'input_consent_required' "$TARGET" \
  'display interactive downgrade must publish missing input consent as a stable reason'
require 'display_interactive_downgrades_until_input_consent_exists' "$TARGET" \
  'target binding tests must prove display interactive does not imply input consent'
require 'target-scoped keyboard/pointer dispatch is unsafe' "$TARGET" \
  'view-only downgrade must document the missing focus-safe validator'
require 'target_scoped_keyboard_pointer_dispatch_unsafe' "$TARGET" \
  'view-only downgrade must publish a stable machine-readable input scope reason'
require '"input_scope_reason": self\.scope_audit\.input_scope_reason\.as_str\(\)' "$TARGET" \
  'target binding and TARGET_BOUND projection must expose the committed input scope reason'
require 'binding\.scope_audit_value\(\)\["input_scope_reason"\]' "$TARGET" \
  'target binding tests must prove input scope reason is externally visible in scope_audit'
require 'binding\.target_bound_event_payload\(\)\["input_scope_reason"\]' "$TARGET" \
  'target binding tests must prove input scope reason is externally visible in TARGET_BOUND'
require 'binding\.target_bound_event_payload\(\)\["consent_epoch"\]' "$TARGET" \
  'target binding tests must prove consent epoch is externally visible in TARGET_BOUND'
require 'application_interactive_downgrade_projects_input_scope_reason' "$TARGET" \
  'target binding tests must prove app/window interactive downgrade reason is visible'
require 'display_interactive_without_input_consent_remains_view_only' "$INPUT" \
  'input policy tests must prove display interactive cannot enable key/pointer without input consent'
require 'fn input_policy_for_scope\(' "$INPUT" \
  'input policy must centralize scope-based disablement'
require 'fn input_policy_reject_reason\(' "$INPUT" \
  'input rejection reason must be centralized for datachannel and bidi paths'
require 'fn reject_unsupported_input_channel_frame\(' "$INPUT" \
  'unsupported rich input frames must be rejected at the input data-channel validation boundary'
require_multiline 'm/fn validate_input_frame\([^}]+?reject_unsupported_input_channel_frame\(frame\)\?/s' "$INPUT" \
  'input frame validation must reject unsupported rich input before policy application'
require 'fn apply_input_frame_with_policy\(' "$INPUT" \
  'input frame application must expose a single policy-enforced application boundary'
require_multiline 'm/fn apply_input_frame_with_policy\([^}]+?input_policy_reject_reason\(input_policy, frame\.kind\(\)\.as_policy_key\(\)\)/s' "$INPUT" \
  'input frame application must enforce centralized input policy before OS injection'
require 'apply_input_frame_with_policy_is_the_policy_enforcement_boundary' "$INPUT" \
  'input tests must prove apply_input_frame_with_policy is the policy enforcement boundary'
require 'parse_input_frame_rejects_clipboard_and_file_drop_before_policy_application' "$INPUT" \
  'input parser tests must prove clipboard/file-drop fail before policy application'
require 'InputScope::ViewOnly => \{' "$INPUT" \
  'view-only input policy branch must exist'
require 'disable_input_policy_key\(&mut map, "keyboard_enabled"\)' "$INPUT" \
  'view-only input policy must disable keyboard'
require 'disable_input_policy_key\(&mut map, "pointer_enabled"\)' "$INPUT" \
  'view-only input policy must disable pointer'
require_multiline 'm/fn input_policy_reject_reason\(.+?input_scope == Some\(InputScope::ViewOnly\.as_str\(\)\).+?return Some\("input_scope_unsupported"\)/s' "$INPUT" \
  'view-only key/pointer rejection must report input_scope_unsupported'
require_multiline 'm/InputRejectSample::new\(\s*outcome\.reason\.unwrap_or\("input_injection_failed"\),\s*rejected_count/s' "$INPUT" \
  'WebRTC input rejection diagnostics must use the policy-enforced apply outcome'
require 'BTreeMap<InputRejectSignature, PendingInputReject>' "$INPUT" \
  'input rejection coalescing must aggregate by signature instead of a single pending rejection'
require 'input_reject_diagnostics_are_coalesced_across_interleaved_signatures' "$INPUT" \
  'PERF-07 must prove alternating invalid input signatures do not produce one diagnostic per frame'
require 'fn current_session_input_policy\(' "$INPUT" \
  'input readiness must be centralized at the session aggregate boundary'
require 'InputTransportGuard::DirectWebRtc\(epoch\)' "$INPUT" \
  'production input path must guard frames by the current WebRTC transport epoch'
require 'current_session_input_policy\(' "$INVOKE_BIDI" \
  'diagnostic bidi input path must re-read session readiness for each input frame'
require 'InputTransportGuard::DiagnosticPreview' "$INVOKE_BIDI" \
  'diagnostic bidi input path must guard frames by preview attachment state'
require 'handle_parsed_bidi_input_frame\(&effective_input_policy' "$INVOKE_BIDI" \
  'diagnostic bidi input path must apply parsed input frames against refreshed policy'
require 'apply_input_frame_with_policy\(input_policy, frame\)' "$INVOKE_BIDI" \
  'diagnostic bidi input path must use the single policy-enforced input application boundary'
reject 'input_policy_reject_reason' "$INVOKE_BIDI" \
  'diagnostic bidi path must not duplicate input policy checks outside apply_input_frame_with_policy'
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
require 'fn input_policy_object\(' "$INPUT" \
  'input policy construction must canonicalize arbitrary caller JSON into a domain-owned object'
require 'input_policy_builder_canonicalizes_non_object_base_policy' "$INPUT" \
  'E2E-11 must prove malformed/non-object base policy cannot bypass canonical view-only enforcement'
require 'input_policy_reports_clipboard_and_file_drop_unsupported_even_when_requested' "$REQUEST" \
  'input parser must report unsupported rich input types explicitly'
require 'UNSUPPORTED_INPUT_CHANNEL_TYPES' "$INPUT" \
  'unsupported rich input types must have one input-domain source of truth'
require 'unsupported_input_channel_types_value\(\)' "$REQUEST" \
  'request input policy projection must reuse the input-domain unsupported type set'
require 'unsupported_input_channel_types_value\(\)' "$VIEW_DEVICE" \
  'device capability metadata must reuse the input-domain unsupported type set'
require '"unsupported_input_types": unsupported_input_channel_types_value\(\)' "$VIEW_DEVICE" \
  'device capabilities must report unsupported input channel types'
require '"unsupported_capabilities":' "$VIEW_DEVICE" \
  'device capabilities must report unsupported rich-input capabilities'
require '"remote_desktop\.clipboard\.write"' "$VIEW_DEVICE" \
  'device capabilities must point clipboard to future split abilities instead of datachannel support'
require '"remote_desktop\.file_transfer\.send"' "$VIEW_DEVICE" \
  'device capabilities must point file transfer to future split abilities instead of datachannel support'
require 'device_capabilities_report_clipboard_and_file_transfer_unsupported' "$VIEW_DEVICE" \
  'device capability tests must prove clipboard/file transfer are reported unsupported'
require 'production_backend\.supported_subjects_value\(\)' "$VIEW_DEVICE" \
  'device capabilities must project production target subjects from the backend descriptor'
require '"production_target_subjects": production_target_subjects' "$VIEW_DEVICE" \
  'device capabilities must expose the native production backend display/window/application subject matrix'
require '"display_scoped_application_window_set"' "$VIEW_DEVICE" \
  'device capabilities must expose the application target model instead of flattening applications to display capture'
require 'display/window/application target capture' "$VIEW_DEVICE" \
  'device capabilities must describe native ScreenCaptureKit as targeted display/window/application capture'
reject 'available for display capture' "$VIEW_DEVICE" \
  'device capabilities must not describe the native targeted backend as display-only'
require 'device_capabilities_project_native_target_subject_matrix' "$VIEW_DEVICE" \
  'device capability tests must prove the native target subject matrix is projected'

printf 'check-remoteapp-lifecycle-input-boundary: ok\n'
