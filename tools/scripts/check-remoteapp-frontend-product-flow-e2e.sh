#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
FRONTEND_ROOT="${CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT:-$ROOT/../EasyNet/Frontend}"
HARNESS="$ROOT/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
BROWSER_LIFECYCLE="$ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
PERMISSION_SUBJECT="$ROOT/tools/scripts/host-remoteapp-permission-subject-e2e.sh"
TARGET_FRESHNESS="$ROOT/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"
DECODED_FRAME="$ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
VIEW_ONLY_INPUT="$ROOT/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"
HUB_API_PREFLIGHT="$ROOT/tools/scripts/hub-api-readiness-preflight.sh"
FRONTEND_UI_TEST="$FRONTEND_ROOT/src/components/easynet/DeviceMediaAccess.test.tsx"
FRONTEND_UI="$FRONTEND_ROOT/src/components/easynet/DeviceMediaAccess.tsx"
FRONTEND_SHARE_PICKER="$FRONTEND_ROOT/src/components/easynet/ShareContentPicker.tsx"
FRONTEND_STORE="$FRONTEND_ROOT/src/store/media-channel-store.ts"
FRONTEND_STORE_TEST="$FRONTEND_ROOT/src/store/media-channel-store.test.ts"
FRONTEND_PROTOCOL="$FRONTEND_ROOT/src/lib/api/remote-desktop-protocol.ts"
REMOTEAPP_NETWORK="$ROOT/plugins/remote-desktop/src/network.rs"
REMOTEAPP_TRANSPORT_VIEW="$ROOT/plugins/remote-desktop/src/view_transport.rs"
AUDIT="$ROOT/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
PLAN="$ROOT/pr/20260822-remoteapp-product-closure/02-evidence-audit.md"

fail() {
  printf 'check-remoteapp-frontend-product-flow-e2e: %s\n' "$1" >&2
  exit 1
}

require() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  rg -q -- "$pattern" "$path" || fail "$message"
}

reject() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  if rg -q -- "$pattern" "$path"; then
    fail "$message"
  fi
}

line_of() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  local line
  line="$(rg -n -- "$pattern" "$path" | head -n 1 | cut -d: -f1 || true)"
  [[ -n "$line" ]] || fail "$message"
  printf '%s\n' "$line"
}

require_order() {
  local before_pattern="$1"
  local after_pattern="$2"
  local path="$3"
  local message="$4"
  local before_line
  local after_line
  before_line="$(line_of "$before_pattern" "$path" "$message")"
  after_line="$(line_of "$after_pattern" "$path" "$message")"
  (( before_line < after_line )) || fail "$message"
}

[[ -f "$HARNESS" ]] || fail "missing frontend RemoteApp product-flow E2E harness"
[[ -x "$HARNESS" ]] || fail "frontend RemoteApp product-flow E2E harness must be executable"
[[ -f "$BROWSER_LIFECYCLE" ]] || fail "missing frontend RemoteApp Browser/Tauri lifecycle E2E verifier"
[[ -x "$BROWSER_LIFECYCLE" ]] || fail "frontend RemoteApp Browser/Tauri lifecycle E2E verifier must be executable"
[[ -f "$PERMISSION_SUBJECT" ]] || fail "missing host permission subject E2E harness"
[[ -f "$TARGET_FRESHNESS" ]] || fail "missing host target picker freshness E2E harness"
[[ -f "$DECODED_FRAME" ]] || fail "missing host decoded-frame E2E harness"
[[ -f "$VIEW_ONLY_INPUT" ]] || fail "missing host view-only input safety E2E harness"
[[ -f "$HUB_API_PREFLIGHT" ]] || fail "missing Hub API readiness preflight harness"
[[ -f "$FRONTEND_UI_TEST" ]] || fail "missing frontend RemoteApp UI flow test"
[[ -f "$FRONTEND_UI" ]] || fail "missing frontend RemoteApp UI component"
[[ -f "$FRONTEND_STORE" ]] || fail "missing frontend RemoteApp media channel store"
[[ -f "$FRONTEND_STORE_TEST" ]] || fail "missing frontend RemoteApp media channel store test"
[[ -f "$FRONTEND_PROTOCOL" ]] || fail "missing frontend RemoteApp protocol projection"
[[ -f "$REMOTEAPP_NETWORK" ]] || fail "missing RemoteApp network route model"
[[ -f "$REMOTEAPP_TRANSPORT_VIEW" ]] || fail "missing RemoteApp transport view projection"
[[ -f "$AUDIT" ]] || fail "missing RemoteApp product readiness audit"
[[ -f "$PLAN" ]] || fail "missing RemoteApp product closure evidence plan"

bash "$HARNESS" --self-test >/dev/null
bash "$BROWSER_LIFECYCLE" --self-test >/dev/null
bash "$HUB_API_PREFLIGHT" --self-test >/dev/null
require 'real_browser_tauri_lifecycle' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require real frontend lifecycle proof mode'
require 'component_mock.*False|component_mock.*false' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must reject component-mock evidence'
require 'real_backend_runtime.*True|real_backend_runtime.*true' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require a real backend/runtime'
require 'product_complete_claim.*False|product_complete_claim.*false' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must reject product-complete claims'
require 'target_picker_opened' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require target picker evidence'
require 'permission_status_checked' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require permission_status preflight evidence'
require 'remote_desktop\.permission_status' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must bind permission preflight to remote_desktop.permission_status'
require 'consent_granted' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require consent grant evidence'
require 'remote_desktop\.grant_consent' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must bind consent to remote_desktop.grant_consent'
require 'session_created' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require session creation evidence'
require 'remote_desktop\.create_session' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must bind creation to remote_desktop.create_session'
require 'webrtc_attached' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require WebRTC attach evidence'
require 'remote_desktop\.attach' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must bind attach to remote_desktop.attach'
require 'watch_events_streaming' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require watch_events evidence'
require 'remote_desktop\.watch_events' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must bind event watch to remote_desktop.watch_events'
require 'media_presented' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require rendered media evidence'
require 'media_pipeline_support_visible' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require visible media pipeline support evidence'
require 'pipeline video_only' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require video-only media pipeline visibility'
require 'bounded_queue_drop_stale_frames' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require stale-frame drop policy visibility'
require 'remoteapp_media_adaptation_e2e_artifact_missing' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require missing media-adaptation E2E blocker visibility'
require 'input_control_attempted_or_policy_blocked' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require input/control or policy-block evidence'
require 'session_ended' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require session end evidence'
require 'remote_desktop\.end_session' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must bind end to remote_desktop.end_session'
require 'terminal_receipt_visible' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require visible terminal receipt evidence'
require 'permission_status_checked must be host-local and not target-scoped' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must keep permission_status host-local'
require '--run requires --evidence-json or --runner-cmd' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require real runner output in --run mode'
require 'EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_E2E' "$BROWSER_LIFECYCLE" \
  'Browser/Tauri lifecycle verifier must require an explicit live-run gate'
require '/api/v1/health' "$HUB_API_PREFLIGHT" \
  'Hub API readiness preflight must probe the canonical backend health endpoint'
require '"connection_state": connection\.get\("state"\)' "$HUB_API_PREFLIGHT" \
  'Hub API readiness preflight must preserve runtime connection state diagnostics'
require '"connection_failure": connection\.get\("failure"\)' "$HUB_API_PREFLIGHT" \
  'Hub API readiness preflight must preserve runtime credential failure diagnostics'
require '"hub_endpoint": connection\.get\("hub_endpoint"\)' "$HUB_API_PREFLIGHT" \
  'Hub API readiness preflight must preserve paired Hub endpoint diagnostics'
require 'details\["preflight_error"\]' "$HUB_API_PREFLIGHT" \
  'Hub API readiness preflight must record runtime-status preflight errors in evidence'
require 'Docker daemon is not reachable' "$HUB_API_PREFLIGHT" \
  'Hub API readiness preflight must classify Docker-daemon unreachability explicitly'
require 'write_report "failed" "Hub API health is not reachable"' "$HUB_API_PREFLIGHT" \
  'Hub API readiness preflight must write a standard report for failed health probes'
require 'does not start Docker' "$HUB_API_PREFLIGHT" \
  'Hub API readiness preflight must stay read-only and never start Docker implicitly'

require 'npx tsc --noEmit' "$HARNESS" \
  'product-flow harness must run frontend TypeScript checks'
require 'npm test -- src/components/easynet/DeviceMediaAccess\.test\.tsx' "$HARNESS" \
  'product-flow harness must run DeviceMediaAccess RemoteApp UI flow coverage'
require 'hub-api-readiness-preflight\.sh' "$HARNESS" \
  'product-flow harness must invoke Hub API readiness preflight'
require 'run_step hub-api-readiness-preflight run_hub_api_readiness_preflight' "$HARNESS" \
  'product-flow harness must execute Hub API readiness as the first product-flow gate'
require 'run_product_runtime_readiness_preflight' "$HARNESS" \
  'product-flow harness must run an explicit product runtime readiness preflight'
require 'run_step product-runtime-readiness-preflight run_product_runtime_readiness_preflight' "$HARNESS" \
  'product-flow harness must execute runtime readiness before frontend and host evidence'
require 'runtime status --json' "$HARNESS" \
  'product-flow runtime readiness preflight must inspect easynet runtime status'
require 'daemon\.control_accepting is not true' "$HARNESS" \
  'product-flow runtime readiness preflight must require daemon control readiness'
require 'daemon\.invocation_accepting is not true' "$HARNESS" \
  'product-flow runtime readiness preflight must require daemon invocation readiness'
require 'connection\.failure=' "$HARNESS" \
  'product-flow runtime readiness preflight must report connection failure codes'
require 'hub_api_endpoint=' "$HARNESS" \
  'product-flow runtime readiness preflight must report the Hub API endpoint used for credential verification'
require_order 'run_step hub-api-readiness-preflight run_hub_api_readiness_preflight' 'run_step product-runtime-readiness-preflight run_product_runtime_readiness_preflight' "$HARNESS" \
  'product-flow must check Hub API before daemon runtime readiness'
require_order 'run_step product-runtime-readiness-preflight run_product_runtime_readiness_preflight' 'run_step frontend-typecheck run_frontend_tsc' "$HARNESS" \
  'product-flow report order must put runtime readiness before frontend checks'
require 'host-remoteapp-permission-subject-e2e\.sh' "$HARNESS" \
  'product-flow harness must invoke host permission subject E2E'
require '--require-screen-capture-granted' "$HARNESS" \
  'product-flow harness must require granted screen-capture permission before decoded-frame E2E'
require 'host-remoteapp-target-picker-freshness-e2e\.sh' "$HARNESS" \
  'product-flow harness must invoke live target picker freshness E2E'
require 'host-remoteapp-decoded-frame-e2e\.sh' "$HARNESS" \
  'product-flow harness must invoke decoded-frame WebRTC E2E'
require 'host-remoteapp-view-only-input-safety-e2e\.sh' "$HARNESS" \
  'product-flow harness must invoke view-only input safety E2E'
require '--sentinel-fixture' "$HARNESS" \
  'product-flow harness must use sentinel fixtures for app/window evidence'
require '--pre-media-resource-refresh' "$HARNESS" \
  'product-flow harness must refresh media resources before decoded-frame evidence'
require '--target-kind "\$kind"' "$HARNESS" \
  'product-flow harness must parameterize decoded-frame/view-only evidence by target kind'
require 'EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E' "$HARNESS" \
  'product-flow harness must require an explicit run gate'
require 'write_json_report "skipped"' "$HARNESS" \
  'product-flow harness must write a skipped report instead of pretending product evidence exists'
require 'write_json_report "failed" "step \$name failed"' "$HARNESS" \
  'product-flow harness must write a top-level failed report when a product-flow step fails'
require '"failed_step"' "$HARNESS" \
  'product-flow harness report must identify the failed step'
require '"steps": steps' "$HARNESS" \
  'product-flow harness report must aggregate step results'
require '"step_order": step_order' "$HARNESS" \
  'product-flow harness report must preserve execution-order step semantics'
require '"stderr_excerpt"' "$HARNESS" \
  'product-flow harness report must preserve bounded stderr excerpts for failure triage'
require 'does not claim product completion' "$HARNESS" \
  'product-flow harness must explicitly avoid product-complete claims'

require 'runs the remote desktop UI flow from target picker through session end' "$FRONTEND_UI_TEST" \
  'frontend component test must cover picker-to-session-end user flow'
require 'watch_events' "$FRONTEND_UI_TEST" \
  'frontend UI flow test must prove watch_events is part of the session lifecycle'
require 'to_client_ice_server_value' "$REMOTEAPP_NETWORK" \
  'RemoteApp network route model must project browser-consumable ICE server config'
require 'DirectWebRtcClientIceServerProjection' "$REMOTEAPP_NETWORK" \
  'RemoteApp network route model must keep client ICE server projection as a typed product object'
require '"client_ice_servers": self\.client_ice_servers\.clone\(\)' "$REMOTEAPP_TRANSPORT_VIEW" \
  'RemoteApp transport view must expose browser client_ice_servers'
require 'client_ice_server_projection_includes_browser_turn_credentials' "$REMOTEAPP_NETWORK" \
  'RemoteApp network tests must prove browser TURN credentials are projected to authorized session views'
require 'client_ice_servers' "$FRONTEND_PROTOCOL" \
  'frontend protocol projection must parse daemon-projected RemoteApp ICE server config'
require 'webrtcIceServers: remoteDesktopIceServersFromValue\(transport\?\.client_ice_servers\)' "$FRONTEND_PROTOCOL" \
  'frontend RemoteApp view must derive browser ICE servers from the session transport view'
require 'iceServers: view\.webrtcIceServers' "$FRONTEND_STORE" \
  'frontend WebRTC startup must use session-projected ICE servers'
reject 'iceServers: \[\]' "$FRONTEND_STORE" \
  'frontend RemoteApp WebRTC startup must not hard-code an empty ICE server list'
require 'remoteDesktopRouteStatusLabel' "$FRONTEND_UI" \
  'frontend RemoteApp UI must render daemon-projected network route state'
require 'transportRouteState' "$FRONTEND_UI" \
  'frontend RemoteApp route UI must derive from daemon transportRouteState'
require 'route host_only · no NAT/relay' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove host-only RemoteApp routes are visible as not NAT/relay ready'
require 'frame\.target_geometry_revision !== expectedRevision' "$FRONTEND_PROTOCOL" \
  'frontend RemoteApp input gating must reject stale or missing pointer target geometry revisions before send'
require 'remoteDesktopInputScopeLabel\(session\)' "$FRONTEND_UI" \
  'frontend RemoteApp UI must render daemon-projected input scope and pointer/keyboard enablement'
require 'input scope display_global · pointer\+keyboard' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove display-global pointer/keyboard enablement is visible in session details'
require 'input scope display_global · no controls' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove blocked input scope is visible as no controls'
require 'remoteDesktopPermissionActionRecommended\(session\)' "$FRONTEND_UI" \
  'frontend RemoteApp UI must offer permission recovery from daemon input permission blockers'
require 'offers permission recovery when daemon input injection is unavailable' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove input-injection blockers expose Request permission recovery'
require 'rdRequestPermission' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove Request permission recovery executes the store action'
require 'onCheckPermission' "$FRONTEND_UI" \
  'frontend RemoteApp panel must wire permission_status preflight into the share picker'
require 'Check permissions' "$FRONTEND_SHARE_PICKER" \
  'frontend share picker must expose a non-prompting RemoteApp permission preflight action'
require 'remote_desktop\.permission_status' "$FRONTEND_UI_TEST" \
  'frontend UI flow tests must prove permission_status is part of the pre-share authorization flow'
require 'keeps the remote desktop picker open after denied permission preflight' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove denied permission_status does not eject the user from the picker'
require 'configures browser WebRTC with session-projected RemoteApp ICE servers' "$FRONTEND_STORE_TEST" \
  'frontend store tests must prove RTCPeerConnection receives session-projected ICE servers'
require 'turn:turn\.example\.test:3478\?transport=udp' "$FRONTEND_STORE_TEST" \
  'frontend store test must cover TURN relay ICE server config, not only host/direct mode'
require 'fails closed before sending stale RemoteApp pointer geometry revisions' "$FRONTEND_STORE_TEST" \
  'frontend store tests must prove stale pointer target geometry revisions do not reach the data channel'
require 'target_geometry_revision: 6' "$FRONTEND_STORE_TEST" \
  'frontend stale pointer test must use an explicit stale geometry revision'
require 'RemoteDesktopAudioSupport' "$FRONTEND_PROTOCOL" \
  'frontend protocol projection must type daemon RemoteApp audio product state'
require 'RemoteDesktopMediaPipelineSupport' "$FRONTEND_PROTOCOL" \
  'frontend protocol projection must type daemon RemoteApp media pipeline support state'
require 'mediaPipelineSupport: remoteDesktopMediaPipelineSupportFromResult\(result\)' "$FRONTEND_PROTOCOL" \
  'frontend RemoteApp view must consume daemon media_pipeline_support projection'
require 'remoteapp_media_adaptation_e2e_artifact_missing' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove missing media-adaptation E2E remains visible as a product blocker'
require 'remoteDesktopMediaPipelineLabel' "$FRONTEND_UI" \
  'frontend RemoteApp UI must render daemon-projected media pipeline support'
require 'pipeline video_only · h264 · bounded_queue_drop_stale_frames · host_audio_not_implemented' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove media pipeline support is visible in session details'
require 'audioReady: readiness\.audio_ready === true' "$FRONTEND_PROTOCOL" \
  'frontend production readiness must parse audio readiness separately from video readiness'
require 'host_audio_not_implemented' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove session details surface the host-audio unsupported blocker'
require 'remoteDesktopMediaQualityLabel' "$FRONTEND_UI" \
  'frontend RemoteApp UI must render a media quality summary from session stats'
require 'rtpSenderBackpressureDrops' "$FRONTEND_UI" \
  'frontend media quality summary must account for RTP sender backpressure drops'
require 'media 18000kbps · 52\.5fps · drops 15 · backpressure 3' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove media bitrate, FPS, drops, and backpressure are visible in session details'
require 'remoteDesktopTargetStatusLabel' "$FRONTEND_UI" \
  'frontend RemoteApp UI must render daemon-projected target recovery state in session details'
require 'remoteDesktopTargetRecoveryMessage' "$FRONTEND_UI" \
  'frontend RemoteApp UI must consume canonical target recovery projection instead of inventing UI-only target state'
require 'latestTargetDiagnostic' "$FRONTEND_UI" \
  'frontend RemoteApp target status must be derived from daemon latestTargetDiagnostic'
require 'frontendAction' "$FRONTEND_UI" \
  'frontend RemoteApp target status must surface daemon frontendAction recovery guidance'
require 'remoteDesktopTargetRecoveryAction' "$FRONTEND_UI" \
  'frontend RemoteApp UI must derive executable recovery actions from daemon target diagnostics'
require 'onRefreshRemoteTargets' "$FRONTEND_UI" \
  'frontend RemoteApp UI must expose a refresh-targets action for daemon refresh_targets guidance'
require 'Refresh targets' "$FRONTEND_UI" \
  'frontend RemoteApp action row must offer a Refresh targets CTA for lost application/window targets'
require 'surfaces daemon remote desktop target recovery state in session details' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove session details surface target recovery state'
require 'target lost · target_not_found · refresh_targets' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove lost window/application targets expose reason and recovery action'
require 'refreshRemoteTargets' "$FRONTEND_UI_TEST" \
  'frontend UI tests must bind the lost-target recovery button to a refresh callback'
require 'toHaveBeenCalledTimes\(1\)' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove the Refresh targets CTA executes the recovery callback'
require 'Retry session' "$FRONTEND_UI" \
  'frontend RemoteApp UI must expose an executable Retry session CTA for daemon retry_session guidance'
require 'remoteDesktopRetrySessionRecommended' "$FRONTEND_UI" \
  'frontend RemoteApp retry CTA must be gated by explicit retry-session state'
require 'executes daemon-requested remote desktop retry through end then create' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove Retry session executes the recovery flow'
require 'rdEnd\.mock\.invocationCallOrder\[0\].*rdCreate\.mock\.invocationCallOrder\[0\]' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove Retry session ends the current session before creating a new one'
require 'entry\.loading \|\| \(entry\.session && !remoteDesktopSessionTerminal\(entry\.session\)\)' "$FRONTEND_STORE" \
  'frontend store must allow create_session after a retained terminal session receipt'
require 'allows creating a new remote desktop session after a terminal receipt is retained' "$FRONTEND_STORE_TEST" \
  'frontend store tests must prove terminal receipts do not block new create_session'

require 'frontend-remoteapp-product-flow-e2e\.sh' "$AUDIT" \
  'product readiness audit must mention the product-flow E2E harness'
require 'frontend-remoteapp-browser-lifecycle-e2e\.sh' "$AUDIT" \
  'product readiness audit must mention the Browser/Tauri lifecycle evidence verifier'
require 'runnable product-flow harness entrypoint' "$AUDIT" \
  'product readiness audit must classify the harness as an entrypoint, not proof of completion'
require 'Live Browser/Tauri E2E artifact with real backend/runtime' "$AUDIT" \
  'product readiness audit must retain real Browser/Tauri full-flow evidence as still required'
require 'visible terminal receipt' "$AUDIT" \
  'product readiness audit must require visible terminal receipt evidence'
require 'visible `media_pipeline_support`' "$AUDIT" \
  'product readiness audit must require visible media_pipeline_support evidence'
require 'target recovery' "$AUDIT" \
  'product readiness audit must record frontend target recovery projection evidence'
require 'route state' "$AUDIT" \
  'product readiness audit must record frontend route-state visibility evidence'
require 'media quality' "$AUDIT" \
  'product readiness audit must record frontend media-quality visibility evidence'
require 'media_pipeline_support' "$AUDIT" \
  'product readiness audit must record frontend media-pipeline support visibility evidence'
require 'Retry session' "$AUDIT" \
  'product readiness audit must record executable retry-session evidence'
require 'input scope display_global' "$AUDIT" \
  'product readiness audit must record input-scope visibility evidence'
require 'Accessibility/input-injection permission' "$AUDIT" \
  'product readiness audit must record input-permission recovery evidence'
require 'permission_status preflight' "$AUDIT" \
  'product readiness audit must record frontend permission preflight evidence'
require 'Denied `permission_status` now remains picker-local' "$PLAN" \
  'product closure plan must record denied permission preflight picker retention'
require 'RemoteApp interactive desktop product: incomplete' "$AUDIT" \
  'product readiness audit must keep product status incomplete'
reject 'RemoteApp interactive desktop product: complete' "$AUDIT" \
  'product readiness audit must not claim product completion'

require 'frontend-remoteapp-product-flow-e2e\.sh' "$PLAN" \
  'product closure plan must mention the product-flow E2E harness'
require 'frontend-remoteapp-browser-lifecycle-e2e\.sh' "$PLAN" \
  'product closure plan must mention the Browser/Tauri lifecycle evidence verifier'
require 'explicit --run report remains required' "$PLAN" \
  'product closure plan must require an explicit run report before using harness evidence'
require 'Frontend full lifecycle E2E across Browser/Tauri surfaces' "$PLAN" \
  'product closure plan must retain Browser/Tauri full lifecycle gap'
require 'visible media pipeline support' "$PLAN" \
  'product closure plan must retain visible media pipeline support as live Browser/Tauri evidence'
require 'target lost · target_not_found · refresh_targets' "$PLAN" \
  'product closure plan must record the target recovery UI evidence'
require 'Refresh targets' "$PLAN" \
  'product closure plan must record the executable target recovery CTA evidence'
require 'route host_only · no NAT/relay' "$PLAN" \
  'product closure plan must record route-state UI evidence'
require 'media 18000kbps · 52\.5fps · drops 15 · backpressure 3' "$PLAN" \
  'product closure plan must record media-quality summary UI evidence'
require 'media_pipeline_support' "$PLAN" \
  'product closure plan must record frontend media-pipeline support visibility evidence'
require 'Retry session' "$PLAN" \
  'product closure plan must record executable retry-session UI evidence'
require 'input scope display_global' "$PLAN" \
  'product closure plan must record input-scope visibility evidence'
require 'Accessibility/input-injection permission' "$PLAN" \
  'product closure plan must record input-permission recovery evidence'
require 'permission_status' "$PLAN" \
  'product closure plan must record frontend permission preflight evidence'

printf 'check-remoteapp-frontend-product-flow-e2e: ok\n'
