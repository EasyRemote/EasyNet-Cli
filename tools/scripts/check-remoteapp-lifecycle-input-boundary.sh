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
SESSION_LIFECYCLE="$REMOTE_ROOT/session_lifecycle.rs"
SESSION_CREATION="$REMOTE_ROOT/session_creation.rs"
INVOKE_BIDI="$REMOTE_ROOT/invoke_bidi.rs"
WEBRTC_ENDPOINT="$REMOTE_ROOT/transport/webrtc_endpoint.rs"
WEBRTC_NEGOTIATION="$REMOTE_ROOT/transport/webrtc_negotiation.rs"

for file in "$TARGET_TRACKING" "$TARGET_OBSERVER" "$TARGET_MONITOR" "$SESSION" "$SESSION_CONSENT_STATE" "$SESSION_IDENTITY" "$RUNTIME" "$CONTRACT" "$SESSION_STATE" "$SESSION_TRANSPORT_STATE" "$SESSION_EVENTS" "$EVENT_LOG" "$VIEW_TRANSPORT" "$VIEW" "$VIEW_DEVICE" "$INPUT" "$TARGET" "$CONSTANTS" "$SCK" "$REQUEST" "$SESSION_STORE" "$CREATE_SESSION" "$SESSION_LIFECYCLE" "$SESSION_CREATION" "$INVOKE_BIDI" "$WEBRTC_ENDPOINT" "$WEBRTC_NEGOTIATION"; do
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
require 'observation_provider_commits_through_session_store_boundary' "$TARGET_OBSERVER" \
  'E2E-08 must test observer-to-session-store geometry commits'
require 'stale_observation_cannot_commit_after_session_binding_reuse' "$TARGET_OBSERVER" \
  'stale observations must not advance a reused session binding'
require 'RemoteDesktopTargetMonitor' "$RUNTIME" \
  'remote desktop runtime must own a plugin-scoped target monitor'
require 'fn track_session_target\(' "$RUNTIME" \
  'remote desktop runtime must expose a target tracking registration boundary'
require 'plugin\.target_monitor\.track\(plugin, session_id\)' "$RUNTIME" \
  'target tracking registration must go through the plugin-owned target monitor'
require 'fn cancel_session_target_tracking\(' "$RUNTIME" \
  'remote desktop runtime must expose a target tracking cancellation boundary'
require 'self\.target_monitor\.cancel\(session_id\)' "$RUNTIME" \
  'target tracking cancellation must go through the plugin-owned target monitor'
require 'RemoteDesktopPlugin::track_session_target\(&plugin, tracker_session_id\)' "$CREATE_SESSION" \
  'create_session must register created sessions with the target monitor'
require 'plugin\.cancel_session_target_tracking\(session_id\)' "$SESSION_LIFECYCLE" \
  'terminal session cleanup must cancel target tracking'
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
require 'lost_observation_returns_media_source_stop_effect_after_debounce' "$TARGET_OBSERVER" \
  'E2E-09 must test observer-debounced target loss returns media stop effect'
require_multiline 'm/fn observe_window\(.+?owner_matches.+?window\.visibility_state != TargetVisibilityState::Visible.+?snapshot\.title\(\).+?snapshot\.focused\(\)/s' "$TARGET_OBSERVER" \
  'window observer must prioritize hidden/minimized availability before title/focus updates'
require 'window_observation_prioritizes_visibility_loss_over_title_or_focus_changes' "$TARGET_OBSERVER" \
  'target observer tests must prove hidden/minimized availability outranks title/focus updates'
require 'MEDIA_SOURCE_LOST' "$SESSION_EVENTS" \
  'session events must project MEDIA_SOURCE_LOST'
require 'binding: &RemoteAppTargetBinding' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST event builder must require committed target binding context'
require '"subject_ura": binding\.subject_ura\(\)' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry subject URA'
require '"binding_id": binding\.binding_id\(\)' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry binding id'
require '"binding_epoch": binding\.binding_epoch\(\)' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry binding epoch'
require '"target_identity_epoch": binding\.target_identity_epoch\(\)' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry target identity epoch'
require '"target_geometry_revision": binding\.target_geometry_revision\(\)' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry target geometry revision'
require '"media_source_epoch": binding\.media_source_epoch\(\)' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST payload must carry media source epoch'
require '"failure_domain": "target"' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST must be a target-domain failure'
require '"media_transport_ready": false' "$SESSION_EVENTS" \
  'MEDIA_SOURCE_LOST must mark media transport unavailable'
require 'events\[media_source_lost_index\]\["binding_id"\]' "$SESSION" \
  'E2E-09 must assert event-log top-level MEDIA_SOURCE_LOST binding id'
require 'events\[media_source_lost_index\]\["target_identity_epoch"\]' "$SESSION" \
  'E2E-09 must assert event-log top-level MEDIA_SOURCE_LOST target identity epoch'
require 'events\[media_source_lost_index\]\["media_source_epoch"\]' "$SESSION" \
  'E2E-09 must assert event-log top-level MEDIA_SOURCE_LOST media source epoch'
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
require '"TARGET_REBIND_FAILED"' "$TARGET_TRACKING" \
  'post-loss target observations must emit an explicit rebind failure when no Rebinding policy exists'
require 'explicit_rebind_required' "$TARGET_TRACKING" \
  'TARGET_REBIND_FAILED must expose explicit_rebind_required'
require 'target_status' "$TARGET_TRACKING" \
  'TARGET_REBIND_FAILED must expose lost target status so frontend cannot treat it as a normal geometry update'
require '"input_enabled": false' "$TARGET_TRACKING" \
  'TARGET_REBIND_FAILED must keep target-scoped input disabled'
require 'tracker_reports_rebind_failure_after_target_loss_without_policy' "$TARGET_TRACKING" \
  'target tracker must test explicit rebind failure instead of silently swallowing post-loss observations'
require 'target_title_after_loss' "$TARGET_TRACKING" \
  'post-loss title observations must enter explicit rebind instead of being silently swallowed'
require 'target_focus_after_loss' "$TARGET_TRACKING" \
  'post-loss focus observations must enter explicit rebind instead of being silently swallowed'
require 'tracker_routes_post_loss_title_focus_through_explicit_rebind' "$TARGET_TRACKING" \
  'target tracker must test title/focus reappearance through explicit rebind semantics'
require 'snapshot_observer_reappearance_requires_explicit_rebind_policy' "$TARGET_OBSERVER" \
  'target observer must prove platform-visible target reappearance cannot revive media/input without explicit rebind policy'
require 'target_reappearance_after_loss_emits_explicit_rebind_failure' "$SESSION" \
  'session aggregate must test post-loss target reappearance as TARGET_REBIND_FAILED'
require_multiline '/"TARGET_REBIND_FAILED"\s*=>\s*"REMOTE_DESKTOP_EVENT_TARGET_CHANGED"/s' "$EVENT_LOG" \
  'event log must project TARGET_REBIND_FAILED as a canonical target change'
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
require 'fn production_readiness_blocked_reason\(session: &RemoteDesktopSession\) -> Value' "$VIEW" \
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
require '"production_ready": session\.production_media_ready\(\)' "$VIEW_TRANSPORT" \
  'transport projection must expose production_ready from the session production predicate'
require '"scope_widened": self\.scope_audit\.scope_widened' "$TARGET" \
  'TARGET_BOUND payload must project scope widening from the committed binding audit'
require '"display_fallback_used": self\.scope_audit\.display_fallback_used' "$TARGET" \
  'TARGET_BOUND payload must project display fallback from the committed binding audit'
require 'target_bound\["payload"\]\["display_fallback_used"\]' "$SESSION" \
  'production readiness test must assert TARGET_BOUND fallback projection'
require '"host_only_no_nat_or_relay"' "$VIEW_TRANSPORT" \
  'host-only transport degradation must have a typed unavailable reason'
require '"relay_unavailable"' "$VIEW_TRANSPORT" \
  'relay-unavailable transport degradation must have a typed unavailable reason'
require 'host_only_candidates_are_not_reported_as_nat_or_relay_ready' "$VIEW_TRANSPORT" \
  'transport tests must prove host-only candidates are not NAT/relay readiness'
require 'easynet_relay_does_not_imply_turn_relay' "$VIEW_TRANSPORT" \
  'transport tests must prove EasyNet relay readiness is distinct from TURN relay readiness'
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

# E2E-11: app/window input remains view-only until a focus-safe target-local
# input validator exists. Pointer geometry may be computed for diagnostics, but
# keyboard/pointer injection is disabled for app/window sessions.
require 'fn input_scope_for_request\(' "$TARGET" \
  'target resolver must centralize requested mode to input scope'
require 'InputScopeDecision' "$TARGET" \
  'target resolver must return an explicit input scope decision, not a bare scope'
require 'RemoteDesktopTargetKind::Window \| RemoteDesktopTargetKind::Application' "$TARGET" \
  'window/application targets must share the view-only input safety branch'
require 'InputScope::ViewOnly' "$TARGET" \
  'app/window requested interactive mode must downgrade to view_only'
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
require 'application_interactive_downgrade_projects_input_scope_reason' "$TARGET" \
  'target binding tests must prove app/window interactive downgrade reason is visible'
require 'fn input_policy_for_scope\(' "$INPUT" \
  'input policy must centralize scope-based disablement'
require 'fn input_policy_reject_reason\(' "$INPUT" \
  'input rejection reason must be centralized for datachannel and bidi paths'
require 'fn apply_input_frame_with_policy\(' "$INPUT" \
  'input frame application must expose a single policy-enforced application boundary'
require_multiline 'm/fn apply_input_frame_with_policy\([^}]+?input_policy_reject_reason\(input_policy, frame\.kind\(\)\.as_policy_key\(\)\)/s' "$INPUT" \
  'input frame application must enforce centralized input policy before OS injection'
require 'apply_input_frame_with_policy_is_the_policy_enforcement_boundary' "$INPUT" \
  'input tests must prove apply_input_frame_with_policy is the policy enforcement boundary'
require 'InputScope::ViewOnly => \{' "$INPUT" \
  'view-only input policy branch must exist'
require 'disable_input_policy_key\(map, "keyboard_enabled"\)' "$INPUT" \
  'view-only input policy must disable keyboard'
require 'disable_input_policy_key\(map, "pointer_enabled"\)' "$INPUT" \
  'view-only input policy must disable pointer'
require_multiline 'm/fn input_policy_reject_reason\(.+?input_scope == Some\(InputScope::ViewOnly\.as_str\(\)\).+?return Some\("input_scope_unsupported"\)/s' "$INPUT" \
  'view-only key/pointer rejection must report input_scope_unsupported'
require_multiline 'm/InputRejectSample::new\(\s*outcome\.reason\.unwrap_or\("input_injection_failed"\),\s*rejected_count/s' "$INPUT" \
  'WebRTC input rejection diagnostics must use the policy-enforced apply outcome'
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

printf 'check-remoteapp-lifecycle-input-boundary: ok\n'
