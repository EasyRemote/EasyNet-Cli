#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_TARGET_BINDING_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
REMOTE_ROOT="$ROOT/plugins/remote-desktop/src"
SPEC="$ROOT/docs/design/remoteapp-targeted-session-spec.md"
SCK_CAPTURE="$REMOTE_ROOT/screencapturekit_capture.rs"
MEDIA="$REMOTE_ROOT/media/mod.rs"

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

require 'ResourceEntryTargetResolver\.resolve_for_session\(' \
  "$REMOTE_ROOT/session_creation.rs" \
  'session creation workflow must be the ResourceEntry-to-target_binding boundary'
require 'verify_target_binding_for_session\(' \
  "$REMOTE_ROOT/session_creation.rs" \
  'session creation workflow must verify the resolved target binding before session insertion'
require 'RemoteDesktopSessionCreationWorkflow::start' \
  "$REMOTE_ROOT/handlers/create_session.rs" \
  'create_session handler must delegate pre-row lifecycle to RemoteDesktopSessionCreationWorkflow'
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
require 'if binding\.target_kind\(\) == RemoteDesktopTargetKind::Display' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC display baseline plan must be guarded as display-only'
require 'RemoteAppMediaSource::DisplayBaseline' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC display baseline must be an explicit media-source selection'
require 'TargetResolutionError::DisplayFallbackForbidden' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC baseline guard must fail app/window sessions with typed display_fallback_forbidden reason'
require_count_at_least 'supported_subjects: &\["display"\]' "$MEDIA" 2 \
  'both xcap baseline media backends must advertise display-only support'
require 'entry\.kind == ResourceType::Display && backend == Some\("xcap"\)' "$MEDIA" \
  'xcap baseline selector must be display-only'
require 'xcap_baseline_catalog_is_display_only_for_remoteapp_targets' "$MEDIA" \
  'media catalog must test xcap baseline does not advertise app/window support'
require 'diagnostic xcap baseline must not advertise app/window capture' "$MEDIA" \
  'xcap display-only test must document that app/window need native target binding'
require_count_at_least '"target_model": self\.target_kind\.target_model\(\)' \
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
require '"target_model": self\.effective_target_kind\.target_model\(\)' \
  "$REMOTE_ROOT/target.rs" \
  'scope audit projection must expose the effective capture target model'
require '"target_model": target_kind\.target_model\(\)' \
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
require 'every_target_resolution_reason_has_canonical_frontend_action_and_axon_context' \
  "$REMOTE_ROOT/target.rs" \
  'target domain must test every canonical target reason projection'
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
require 'input_policy_for_binding\(' \
  "$REMOTE_ROOT/transport/webrtc_negotiation.rs" \
  'WebRTC input policy must consume RemoteAppTargetBinding'
require 'target_binding\(\)' \
  "$REMOTE_ROOT/handlers/attach.rs" \
  'diagnostic attach must consume the session-owned target binding'
require 'off_display_window_ids' \
  "$REMOTE_ROOT/screencapturekit_capture.rs" \
  'ScreenCaptureKit application binding must detect app windows outside the selected display'
require 'TargetResolutionError::TargetMultiDisplayUnsupported' \
  "$REMOTE_ROOT/screencapturekit_capture.rs" \
  'ScreenCaptureKit application binding must use the typed multi-display unsupported reason when single-stream AppSurface would hide other app windows'
require 'MultiAppSurface support' \
  "$REMOTE_ROOT/screencapturekit_capture.rs" \
  'ScreenCaptureKit application binding must explain that multi-display applications require MultiAppSurface support'

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
