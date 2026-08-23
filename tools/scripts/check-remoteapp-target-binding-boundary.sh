#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_TARGET_BINDING_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
REMOTE_ROOT="$ROOT/plugins/remote-desktop/src"
SPEC="$ROOT/docs/design/remoteapp-targeted-session-spec.md"
SCK_CAPTURE="$REMOTE_ROOT/screencapturekit_capture.rs"
MEDIA="$REMOTE_ROOT/media/mod.rs"
TARGET_DOMAIN="$REMOTE_ROOT/target.rs"
SESSION="$REMOTE_ROOT/session.rs"
SESSION_IDENTITY="$REMOTE_ROOT/session_identity.rs"
MEDIA_SOURCE_FACTORY="$REMOTE_ROOT/transport/media_source.rs"

fail() {
  printf 'check-remoteapp-target-binding-boundary: %s\n' "$1" >&2
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

require_count_at_least() {
  local pattern="$1"
  local path="$2"
  local minimum="$3"
  local message="$4"
  local count
  count="$(rg -o -- "$pattern" "$path" | wc -l | tr -d ' ')"
  if (( count < minimum )); then
    fail "$message"
  fi
}

reject() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  if rg -q -- "$pattern" "$path"; then
    fail "$message"
  fi
}

[[ -d "$REMOTE_ROOT" ]] || fail "missing remote desktop source root"
[[ -f "$SPEC" ]] || fail "missing remoteapp targeted session SPEC"

require 'E2E-05 stale window fail-closed' "$SPEC" \
  'SPEC must retain stale window fail-closed acceptance'
require 'E2E-06 no media re-resolution' "$SPEC" \
  'SPEC must retain no media re-resolution acceptance'
require 'E2E-10 weak identity ambiguity' "$SPEC" \
  'SPEC must retain weak identity ambiguity acceptance'

is_test_context() {
  local file="$1"
  local line="$2"
  awk -v line="$line" '
    NR > line { exit(test_context ? 0 : 1) }
    /#\[cfg\(test\)\]/ { saw_cfg_test = 1; next }
    saw_cfg_test && /mod tests/ { test_context = 1; saw_cfg_test = 0; next }
    saw_cfg_test && $0 !~ /^[[:space:]]*$/ { saw_cfg_test = 0 }
    END {
      if (NR <= line) {
        exit(test_context ? 0 : 1)
      }
    }
  ' "$file"
}

# SPEC: after target_binding exists, production media/input paths must not
# convert ResourceEntry directly into native capture or input targets.
reject 'target_for_entry\s*\(' "$REMOTE_ROOT" \
  'production must not resolve native capture targets from ResourceEntry'
reject 'input_policy_for_entry\s*\(' "$REMOTE_ROOT" \
  'production input policy must consume RemoteAppTargetBinding'
reject 'pointer_target_for_entry\s*\(' "$REMOTE_ROOT" \
  'production pointer targeting must consume RemoteAppTargetBinding plus tracker snapshot'

require 'ResourceEntryTargetResolver' \
  "$REMOTE_ROOT/session_creation.rs" \
  'session creation workflow must own the ResourceEntry-to-target_binding boundary'
require 'resolve_for_session_with_input_consent\(' \
  "$REMOTE_ROOT/session_creation.rs" \
  'session creation workflow must be the ResourceEntry-to-target_binding boundary with explicit input-control consent'
require 'input_control_granted' \
  "$REMOTE_ROOT/session_creation.rs" \
  'session creation workflow must make input-control consent explicit before target binding resolution'
require 'verify_target_binding_for_session\(' \
  "$REMOTE_ROOT/session_creation.rs" \
  'session creation workflow must verify the resolved target binding before session insertion'
require 'RemoteDesktopSessionCreationWorkflow::start' \
  "$REMOTE_ROOT/handlers/create_session.rs" \
  'create_session handler must delegate pre-row lifecycle to RemoteDesktopSessionCreationWorkflow'
reject 'subject_type:\s*ResourceType' "$SESSION_IDENTITY" \
  'session profile must not cache subject_type; public subject_type must derive from the committed target binding'
reject 'subject_type:\s*target_binding\.target_kind\(\)\.resource_type\(\)' "$SESSION_IDENTITY" \
  'session identity must not duplicate target kind from the committed binding'
require_multiline 'm/fn subject_type\(&self\) -> ResourceType\s*\{\s*self\.target\.binding\(\)\.target_kind\(\)\.resource_type\(\)\s*\}/s' \
  "$SESSION" \
  'public subject_type projection must derive from RemoteAppTargetBinding, not session profile state'
require 'create_session_rejects_stale_window_inventory_before_session_insert' \
  "$REMOTE_ROOT/handlers/create_session.rs" \
  'E2E-05 must have a stale window create_session fail-closed test'
require 'target_not_found' \
  "$REMOTE_ROOT/handlers/create_session.rs" \
  'E2E-05 stale target failure must expose target_not_found'
require 'frontend_action=refresh_targets' \
  "$REMOTE_ROOT/handlers/create_session.rs" \
  'E2E-05 stale target failure must return refresh_targets frontend action'
require '!sessions\.contains_key\("rd-stale-window"\)' \
  "$REMOTE_ROOT/handlers/create_session.rs" \
  'E2E-05 stale target failure must prove no active session row is inserted'
require 'create_session_rejects_weak_window_identity_before_session_insert' \
  "$REMOTE_ROOT/handlers/create_session.rs" \
  'E2E-10 must have a weak window identity create_session fail-closed test'
require 'target_identity_ambiguous' \
  "$REMOTE_ROOT/handlers/create_session.rs" \
  'E2E-10 weak identity failure must expose target_identity_ambiguous'
require '!sessions\.contains_key\("rd-weak-window"\)' \
  "$REMOTE_ROOT/handlers/create_session.rs" \
  'E2E-10 weak target identity failure must prove no active session row is inserted'
require 'session\.target_binding\(\)\.clone\(\)' \
  "$REMOTE_ROOT/transport/webrtc_negotiation.rs" \
  'WebRTC negotiation must consume the session-owned target binding'
require 'binding\.require_capture_proof\(ABILITY_SET_DESCRIPTION\)' \
  "$MEDIA_SOURCE_FACTORY" \
  'direct WebRTC media source startup must require a committed capture proof before selecting a media source'
require 'validate_available_webrtc_backend\(request\.config\.backend, binding\)\?' \
  "$MEDIA_SOURCE_FACTORY" \
  'direct WebRTC media source startup must validate backend availability and transport compatibility at the factory boundary'
require 'fn validate_available_webrtc_backend\(' \
  "$MEDIA_SOURCE_FACTORY" \
  'direct WebRTC media source factory must own backend availability validation'
require '!backend\.is_available\(\) \|\| !backend\.is_webrtc_transport\(\) \|\| !backend\.transport_ready\(\)' \
  "$MEDIA_SOURCE_FACTORY" \
  'direct WebRTC media source factory must reject unavailable or non-WebRTC backend descriptors before media startup'
require 'validate_native_production_binding\(request\.config\.backend, binding\)\?' \
  "$MEDIA_SOURCE_FACTORY" \
  'direct WebRTC production media source startup must validate backend subject/native binding compatibility at the factory boundary'
require 'fn validate_native_production_binding\(' \
  "$MEDIA_SOURCE_FACTORY" \
  'direct WebRTC media source factory must own native production binding validation'
require '!backend\.supports_subject\(binding\.target_kind\(\)\.resource_type\(\)\)' \
  "$MEDIA_SOURCE_FACTORY" \
  'production WebRTC backend descriptors must be revalidated against the committed target binding subject'
require 'direct_factory_rejects_uncommitted_target_binding_before_media_selection' \
  "$MEDIA_SOURCE_FACTORY" \
  'media source factory must test that uncommitted target bindings cannot start media'
require 'target_for_binding\(' \
  "$REMOTE_ROOT/transport/webrtc_native_media.rs" \
  'native media must start from RemoteAppTargetBinding'
require 'fn target_for_binding\(' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit media startup must expose target_for_binding as the native binding boundary'
require 'binding\.validate_reverified_capture_proof\(ability, target\.capture_proof\(\)\)' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit target_for_binding must validate the live capture proof against the committed session binding'
require 'fn capture_proof\(&self\) -> &ResolvedCaptureTargetProof' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit resolved target must expose a typed capture proof for binding revalidation'
require 'binding\.committed_app_window_set\(\)' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit application capture must start from the committed AppWindowSetProof'
require 'committed_window_set\.contains_window_id\(window_id\)' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit application capture must not include uncommitted same-app windows'
require 'uncommitted_same_display_windows' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit application capture must collect same-display app windows outside the committed set'
require 'uncommitted_same_display_windows\.push\(window\)' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit application capture must pass uncommitted same-app windows to exceptingWindows'
require 'NSArray::from_slice\(&excepting_window_refs\)' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit application capture must build a native exceptingWindows array from uncommitted windows'
require '&app_window_set\.excepting_windows' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit application capture must use committed-window-set exclusions in the content filter'
reject 'let excepting_windows: Retained<NSArray<SCWindow>> = NSArray::new\(\)' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit application capture must not pass empty exceptingWindows for application sessions'
require 'committed_window_set\.missing_window_ids\(&window_ids\)' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit application capture must fail closed when committed windows disappear'
require 'TargetResolutionError::TargetIdentityChanged' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit application committed-window drift must remain a typed target-domain failure'
require 'fn contains_window_id\(' \
  "$TARGET_DOMAIN" \
  'AppWindowSetProof must expose a read-only committed-window membership helper'
require 'fn missing_window_ids\(' \
  "$TARGET_DOMAIN" \
  'AppWindowSetProof must expose committed-window disappearance evidence'
require 'trait RemoteAppMediaSourceFactory' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC media source selection must use the RemoteAppMediaSourceFactory boundary'
require 'fn start_from_binding\(' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC media source selection must start from RemoteAppTargetBinding'
require 'fn start_remote_app_media_source\(' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC media source selection must expose an injectable factory boundary'
require 'fake_factory_receives_session_owned_binding_without_resource_re_resolution' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'E2E-06 must prove fake media-source factory receives the stored binding_id'
require 'seen_binding_id' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'E2E-06 fake media-source factory must record the committed session binding_id'
require 'Some\(expected_binding_id\)' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'E2E-06 fake media-source factory must assert the stored binding_id is preserved'
require 'start_remote_app_media_source\(' \
  "$REMOTE_ROOT/transport/webrtc_media.rs" \
  'direct WebRTC media loop must call through the injectable media-source boundary'
require 'binding\.supports_xcap_adapter\(\)' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC xcap baseline must require an exact target adapter'
require 'RemoteAppMediaSource::XcapBaseline' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC xcap baseline must be an explicit media-source selection'
require 'without widening its scope' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC xcap baseline failure must state that target scope may not widen'
require_count_at_least 'supported_subjects: &\["display", "window", "application"\]' "$MEDIA" 2 \
  'both xcap media backends must advertise their exact target subject matrix'
require 'screen_target_metadata_resolvable\(entry\)' "$MEDIA" \
  'xcap app/window selection must require resolvable target metadata'
require 'xcap_baseline_catalog_supports_exact_window_and_application_targets' "$MEDIA" \
  'media catalog must test exact xcap app/window support'
require 'direct_webrtc_binding_uses_xcap_without_widening_window_or_application_scope' "$MEDIA" \
  'xcap binding test must prove app/window scope is preserved'
require 'capture_application_rgb_with_xcap' \
  "$ROOT/src/daemon/ability/builtins/resources/media/screen_snapshot.rs" \
  'xcap application media must capture the committed application window set'
require 'MAX_APPLICATION_COMPOSITE_PIXELS' \
  "$ROOT/src/daemon/ability/builtins/resources/media/screen_snapshot.rs" \
  'xcap application compositor must have an explicit memory bound'
require 'application_compositor_cross_display_gap_is_black_not_host_display_content' \
  "$ROOT/src/daemon/ability/builtins/resources/media/screen_snapshot.rs" \
  'xcap application compositor must prove cross-display gaps cannot leak host display pixels'
require_count_at_least '"target_model": self\.target_kind\.target_model_for_platform\(&self\.platform\)' \
  "$REMOTE_ROOT/target.rs" \
  2 \
  'target binding projection and target-bound event must expose the concrete capture target model'
require 'validate_resource_inventory_state\(' \
  "$REMOTE_ROOT/target.rs" \
  'target resolver must fail closed on unavailable or stale live inventory rows'
require 'fn validate_owner_agent_ura\(' \
  "$REMOTE_ROOT/target.rs" \
  'target resolver must centralize resource owner_agent Agent/SystemAgent URA validation'
require 'validate_owner_agent_ura\(ability, entry\)\?' \
  "$REMOTE_ROOT/target.rs" \
  'target resolver must reject non-Agent owner_agent before committing target binding'
require 'owner_agent must be an Agent/SystemAgent URA' \
  "$REMOTE_ROOT/target.rs" \
  'owner_agent rejection must explain the owner projection rule'
require 'target_binding_rejects_non_agent_owner_projection' \
  "$REMOTE_ROOT/target.rs" \
  'owner projection rule must have resolver regression coverage'
require 'metadata_freshness_u64\(' \
  "$REMOTE_ROOT/target.rs" \
  'target resolver must consume inventory freshness when present'
require '"target_model": self\.effective_target_kind\.target_model_for_platform\(platform\)' \
  "$REMOTE_ROOT/target.rs" \
  'scope audit projection must expose the effective capture target model'
require '"target_model": target_kind\.target_model_for_platform\(&platform\)' \
  "$REMOTE_ROOT/target.rs" \
  'target resolver diagnostic must expose the resolved capture target model'
require 'display_scoped_application_window_set' \
  "$REMOTE_ROOT/target.rs" \
  'application projection must identify the display-scoped application window-set model'
require 'window_requires_stable_owner_identity_not_app_name_only' \
  "$REMOTE_ROOT/target.rs" \
  'E2E-10 must prove window app_name/title hints are not accepted as stable routing identity'
require 'application_requires_display_scoped_stable_identity' \
  "$REMOTE_ROOT/target.rs" \
  'E2E-10 must prove application app_name-only identity is ambiguous'
require 'TargetResolutionError::TargetIdentityAmbiguous' \
  "$REMOTE_ROOT/target.rs" \
  'E2E-10 target resolver must use the canonical target_identity_ambiguous reason'
require 'app_name/title are diagnostic hints, not production routing identity' \
  "$REMOTE_ROOT/target.rs" \
  'E2E-10 window resolver must document that app_name/title are diagnostic only'
require 'app_name alone is not production routing identity' \
  "$REMOTE_ROOT/target.rs" \
  'E2E-10 application resolver must document that app_name alone is diagnostic only'
require 'fn frontend_action\(self\) -> FrontendAction' \
  "$REMOTE_ROOT/target.rs" \
  'canonical target failures must map to frontend recovery actions in the target domain'
require 'fn target_event_type' \
  "$REMOTE_ROOT/target.rs" \
  'canonical target failures must map to SPEC lifecycle event taxonomy when applicable'
require 'ALL_TARGET_RESOLUTION_ERRORS' \
  "$REMOTE_ROOT/target.rs" \
  'canonical target failure reasons must be maintained as one explicit table'
require 'ALL_FRONTEND_ACTIONS' \
  "$REMOTE_ROOT/target.rs" \
  'frontend recovery actions must be maintained as one explicit table'
require 'with_context\("target_reason", self\.reason\.as_str\(\)\)' \
  "$REMOTE_ROOT/target.rs" \
  'RemoteAppTargetError must project target_reason into Axon context'
require 'with_context\("frontend_action", self\.reason\.frontend_action\(\)\.as_str\(\)\)' \
  "$REMOTE_ROOT/target.rs" \
  'RemoteAppTargetError must project frontend_action into Axon context'
require 'with_context\("target_event_type", target_event_type\)' \
  "$REMOTE_ROOT/target.rs" \
  'RemoteAppTargetError must project SPEC target_event_type into Axon context when available'
require 'every_target_resolution_reason_has_canonical_frontend_action_and_axon_context' \
  "$REMOTE_ROOT/target.rs" \
  'target domain must test every canonical target reason projection'
require 'target_resolution_reasons_project_spec_event_taxonomy_for_create_session_failures' \
  "$REMOTE_ROOT/target.rs" \
  'target domain must test create_session failure reasons against SPEC event taxonomy'
require 'Self::TargetStale => Some\("CAPTURE_TARGET_STALE"\)' \
  "$REMOTE_ROOT/target.rs" \
  'target_stale must project CAPTURE_TARGET_STALE for frontend lifecycle classification'
require 'Self::TargetIdentityAmbiguous => Some\("CAPTURE_TARGET_AMBIGUOUS"\)' \
  "$REMOTE_ROOT/target.rs" \
  'target_identity_ambiguous must project CAPTURE_TARGET_AMBIGUOUS for frontend lifecycle classification'
require 'Self::DisplayFallbackForbidden => Some\("DISPLAY_FALLBACK_FORBIDDEN"\)' \
  "$REMOTE_ROOT/target.rs" \
  'display_fallback_forbidden must project DISPLAY_FALLBACK_FORBIDDEN for frontend lifecycle classification'
require 'Self::TargetPermissionMissing => Some\("SCREEN_CAPTURE_PERMISSION_DENIED"\)' \
  "$REMOTE_ROOT/target.rs" \
  'target_permission_missing must project SCREEN_CAPTURE_PERMISSION_DENIED for frontend lifecycle classification'
require 'downcast_ref::<RemoteAppTargetError>' \
  "$REMOTE_ROOT/registration.rs" \
  'remote desktop registration must preserve typed target failures instead of string parsing'
require 'downcast_ref::<RemoteAppTargetError>' \
  "$REMOTE_ROOT/transport/webrtc_media.rs" \
  'WebRTC media failure projection must preserve typed target failures instead of string parsing'
require 'reason\.frontend_action\(\)\.as_str\(\)' \
  "$REMOTE_ROOT/session_events.rs" \
  'session events must project target failure frontend_action from typed reason'
require 'media_source_loss_projects_typed_frontend_action' \
  "$REMOTE_ROOT/session_events.rs" \
  'session events must test typed target failure frontend_action projection'
require_multiline 'm/EffectiveRemoteDesktopInputPolicy::for_binding\(\s*&input_policy,\s*&target_binding,?\s*\)/s' \
  "$REMOTE_ROOT/transport/webrtc_negotiation.rs" \
  'WebRTC input policy must derive its typed execution policy from the session-owned RemoteAppTargetBinding'
require 'target_binding\(\)' \
  "$REMOTE_ROOT/handlers/attach.rs" \
  'diagnostic attach must consume the session-owned target binding'
reject 'require_diagnostic_preview_supported' "$REMOTE_ROOT" \
  'diagnostic attach support must be selected by a binding-owned frame source, not a target-layer display-only guard'
require 'capture_binding_diagnostic_jpeg\(' \
  "$REMOTE_ROOT/invoke_bidi.rs" \
  'diagnostic InvokeBidi frame loop must capture through a target_binding-owned frame source'
require 'capture_native_binding_diagnostic_jpeg\(' \
  "$REMOTE_ROOT/invoke_bidi.rs" \
  'diagnostic InvokeBidi app/window path must select a native binding-backed adapter'
reject 'capture_subject: DiagnosticCaptureSubject' \
  "$REMOTE_ROOT/invoke_bidi.rs" \
  'diagnostic InvokeBidi frame loop must not carry a ResourceEntry-shaped capture subject as its media boundary'
require 'diagnostic_jpeg_window_capture_does_not_use_resource_entry_backend' \
  "$REMOTE_ROOT/invoke_bidi.rs" \
  'diagnostic InvokeBidi must test that app/window capture does not fall back to the ResourceEntry backend'
require 'struct NativeAppIdentityExpectation\b' \
  "$TARGET_DOMAIN" \
  'target domain must centralize native app identity expectations'
require 'struct NativeAppIdentityCandidate\b' \
  "$TARGET_DOMAIN" \
  'target domain must centralize observed native app identity candidates'
require 'fn app_identity_expectation' \
  "$TARGET_DOMAIN" \
  'native target locator must expose a canonical app identity expectation'
require 'fn native_app_identity_candidate\(&self\)' \
  "$TARGET_DOMAIN" \
  'capture proof validation must expose a canonical native app identity candidate'
require 'native_app_identity_expectation_matches_canonical_bundle_aliases' \
  "$TARGET_DOMAIN" \
  'target domain must test canonical bundle/app identity alias matching'
require 'native_app_identity_expectation_requires_all_declared_identity_fields' \
  "$TARGET_DOMAIN" \
  'target domain must test complete native app identity matching semantics'
require 'capture_proof_revalidation_uses_native_app_identity_aliases' \
  "$TARGET_DOMAIN" \
  'capture proof revalidation must test canonical native app identity alias matching'
require_multiline 'm/fn validate_for_binding\(.+?app_identity_expectation\(\)\s*\.evaluate\(self\.native_app_identity_candidate\(\)\)\s*\.matched\(\)/s' \
  "$TARGET_DOMAIN" \
  'capture proof validation must consume the centralized native app identity matcher'
require_multiline 'm/fn matches_committed_identity\(.+?native_app_identity_expectation\(\)\s*\.evaluate\(self\.native_app_identity_candidate\(\)\)\s*\.matched\(\)/s' \
  "$TARGET_DOMAIN" \
  'committed proof revalidation must consume the centralized native app identity matcher'
require 'fn capture_jpeg_for_binding\(' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit must expose a binding-backed one-shot diagnostic capture adapter'
require 'sck_app_identity_match' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit selectors must consume the centralized native app identity matcher'
require 'app_identity_expectation\(\)' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit selectors must derive identity expectations from the committed target binding'
reject 'expected_pid' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit selectors must not redeclare pid matching outside the target-domain identity matcher'
reject 'expected_bundle_id' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit selectors must not redeclare bundle matching outside the target-domain identity matcher'
reject 'expected_app_identity' \
  "$SCK_CAPTURE" \
  'ScreenCaptureKit selectors must not redeclare app identity matching outside the target-domain identity matcher'
require 'off_display_window_ids' \
  "$REMOTE_ROOT/screencapturekit_capture.rs" \
  'ScreenCaptureKit application binding must detect app windows outside the selected display'
require 'TargetResolutionError::TargetMultiDisplayUnsupported' \
  "$REMOTE_ROOT/screencapturekit_capture.rs" \
  'ScreenCaptureKit application binding must use the typed multi-display unsupported reason when single-stream AppSurface would hide other app windows'
require 'MultiAppSurface support' \
  "$REMOTE_ROOT/screencapturekit_capture.rs" \
  'ScreenCaptureKit application binding must explain that multi-display applications require MultiAppSurface support'
require 'application_window_set_selector_excludes_uncommitted_same_display_windows' \
  "$REMOTE_ROOT/screencapturekit_capture.rs" \
  'ScreenCaptureKit application filter must have regression coverage for excluding uncommitted same-app windows'

while IFS=: read -r file line _match; do
  case "${file#"$ROOT/"}" in
    plugins/remote-desktop/src/target.rs|\
    plugins/remote-desktop/src/handlers/create_session.rs|\
    plugins/remote-desktop/src/session_creation.rs|\
    plugins/remote-desktop/src/test_support.rs)
      continue
      ;;
  esac
  if awk -v line="$line" '
      BEGIN { found = 0 }
      NR >= line - 8 && NR < line && /#\[cfg\(test\)\]/ { found = 1 }
      END { exit(found ? 0 : 1) }
    ' "$file"; then
    continue
  fi
  if is_test_context "$file" "$line"; then
    continue
  fi
  fail "${file#"$ROOT/"}:$line uses ResourceEntryTargetResolver outside the creation/test boundary"
done < <(rg -n -- 'ResourceEntryTargetResolver' "$REMOTE_ROOT" || true)

while IFS=: read -r file line _match; do
  if awk -v line="$line" '
      BEGIN { found = 0 }
      NR >= line - 3 && NR < line && /#\[cfg\(test\)\]/ { found = 1 }
      END { exit(found ? 0 : 1) }
    ' "$file"; then
    continue
  fi
  fail "${file#"$ROOT/"}:$line declares entry-based backend selection outside #[cfg(test)]"
done < <(rg -n -- 'fn (production_backend_for_entry|webrtc_transport_backend_for_entry|select_builtin_h264_backend)\s*\(' "$REMOTE_ROOT" || true)

printf 'check-remoteapp-target-binding-boundary: ok\n'
