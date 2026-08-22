#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
FRONTEND_ROOT="${CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT:-$ROOT/../EasyNet/Frontend}"
HARNESS="$ROOT/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
PERMISSION_SUBJECT="$ROOT/tools/scripts/host-remoteapp-permission-subject-e2e.sh"
TARGET_FRESHNESS="$ROOT/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"
DECODED_FRAME="$ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
VIEW_ONLY_INPUT="$ROOT/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"
HUB_API_PREFLIGHT="$ROOT/tools/scripts/hub-api-readiness-preflight.sh"
FRONTEND_UI_TEST="$FRONTEND_ROOT/src/components/easynet/DeviceMediaAccess.test.tsx"
FRONTEND_UI="$FRONTEND_ROOT/src/components/easynet/DeviceMediaAccess.tsx"
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
bash "$HUB_API_PREFLIGHT" --self-test >/dev/null
require '/api/v1/health' "$HUB_API_PREFLIGHT" \
  'Hub API readiness preflight must probe the canonical backend health endpoint'
require 'Docker daemon is not reachable' "$HUB_API_PREFLIGHT" \
  'Hub API readiness preflight must classify Docker-daemon unreachability explicitly'
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
require 'frame\.target_geometry_revision !== expectedRevision' "$FRONTEND_PROTOCOL" \
  'frontend RemoteApp input gating must reject stale or missing pointer target geometry revisions before send'
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
require 'audioReady: readiness\.audio_ready === true' "$FRONTEND_PROTOCOL" \
  'frontend production readiness must parse audio readiness separately from video readiness'
require 'host_audio_not_implemented' "$FRONTEND_UI_TEST" \
  'frontend UI tests must prove session details surface the host-audio unsupported blocker'
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

require 'frontend-remoteapp-product-flow-e2e\.sh' "$AUDIT" \
  'product readiness audit must mention the product-flow E2E harness'
require 'runnable product-flow harness entrypoint' "$AUDIT" \
  'product readiness audit must classify the harness as an entrypoint, not proof of completion'
require 'Browser/Tauri E2E for full user flow with real backend/runtime' "$AUDIT" \
  'product readiness audit must retain real Browser/Tauri full-flow evidence as still required'
require 'target recovery' "$AUDIT" \
  'product readiness audit must record frontend target recovery projection evidence'
require 'RemoteApp interactive desktop product: incomplete' "$AUDIT" \
  'product readiness audit must keep product status incomplete'
reject 'RemoteApp interactive desktop product: complete' "$AUDIT" \
  'product readiness audit must not claim product completion'

require 'frontend-remoteapp-product-flow-e2e\.sh' "$PLAN" \
  'product closure plan must mention the product-flow E2E harness'
require 'explicit --run report remains required' "$PLAN" \
  'product closure plan must require an explicit run report before using harness evidence'
require 'Frontend full lifecycle E2E across Browser/Tauri surfaces' "$PLAN" \
  'product closure plan must retain Browser/Tauri full lifecycle gap'
require 'target lost · target_not_found · refresh_targets' "$PLAN" \
  'product closure plan must record the target recovery UI evidence'
require 'Refresh targets' "$PLAN" \
  'product closure plan must record the executable target recovery CTA evidence'

printf 'check-remoteapp-frontend-product-flow-e2e: ok\n'
