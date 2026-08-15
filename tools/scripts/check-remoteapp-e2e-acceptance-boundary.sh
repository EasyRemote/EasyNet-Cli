#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
SCRIPT="$ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
PROBE="$ROOT/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
FIXTURE="$ROOT/tools/scripts/host-remoteapp-sentinel-fixture.sh"
RECEIVER="$ROOT/examples/easynet-remoteapp-frame-receiver.rs"
SPEC="$ROOT/docs/design/remoteapp-targeted-session-spec.md"

fail() {
  printf 'check-remoteapp-e2e-acceptance-boundary: %s\n' "$1" >&2
  exit 1
}

require() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  rg -q -- "$pattern" "$path" || fail "$message"
}

[[ -f "$SCRIPT" ]] || fail "missing host decoded-frame E2E harness"
[[ -f "$PROBE" ]] || fail "missing bundled host decoded-frame probe"
[[ -f "$FIXTURE" ]] || fail "missing bundled host sentinel fixture"
[[ -f "$RECEIVER" ]] || fail "missing bundled host decoded-frame receiver"
[[ -f "$SPEC" ]] || fail "missing remoteapp targeted session SPEC"

require 'E2E-03 exact window session' "$SPEC" \
  'SPEC must retain exact window decoded-frame acceptance'
require 'E2E-04 exact application session' "$SPEC" \
  'SPEC must retain exact application decoded-frame acceptance'
require 'E2E-07 display fallback forbidden' "$SPEC" \
  'SPEC must retain display fallback decoded-frame acceptance'

require 'resource\.refresh_remote_targets' "$SCRIPT" \
  'host decoded-frame E2E must prove live refresh inventory was used'
require 'resource\.watch_remote_targets' "$SCRIPT" \
  'host decoded-frame E2E must allow streaming live inventory evidence'
require 'remote_desktop\.create_session' "$SCRIPT" \
  'host decoded-frame E2E must prove remote_desktop.create_session was invoked'
require 'Invocation\.subject|invocation\.subject_ura' "$SCRIPT" \
  'host decoded-frame E2E must validate selected resource URA as Invocation.subject'
require 'WindowSurface' "$SCRIPT" \
  'host decoded-frame E2E must distinguish exact window capture'
require 'window target must include target_binding\.resolved_identity' "$SCRIPT" \
  'host decoded-frame E2E must require window resolved identity evidence'
require 'resolved_identity\.get\("window_id"\)|window_id = resolved_identity\.get\("window_id"\)' "$SCRIPT" \
  'host decoded-frame E2E must require exact native window id evidence'
require 'window evidence must bind selected sentinel pid to resolved_identity\.pid or owner_pid' "$SCRIPT" \
  'host decoded-frame E2E must bind window sentinel pid to resolved owner identity when provided'
require 'AppSurface' "$SCRIPT" \
  'host decoded-frame E2E must distinguish exact application capture'
require 'app_window_set = get\("target_binding\.app_window_set"\)' "$SCRIPT" \
  'host decoded-frame E2E must require application window-set evidence'
require 'resolved_window_ids' "$SCRIPT" \
  'host decoded-frame E2E must require application resolved window ids'
require 'window_set_epoch' "$SCRIPT" \
  'host decoded-frame E2E must require application window-set epoch'
require 'resolved_identity' "$SCRIPT" \
  'host decoded-frame E2E must require application resolved identity evidence'
require 'transport\.kind.*webrtc|transport_kind.*webrtc|"webrtc"' "$SCRIPT" \
  'host decoded-frame E2E must validate WebRTC transport evidence'
require 'production_media_ready' "$SCRIPT" \
  'host decoded-frame E2E must validate post-negotiation production media readiness'
require 'production_readiness\.production_codec_negotiated|production_codec_negotiated' "$SCRIPT" \
  'host decoded-frame E2E must prove a production codec was negotiated'
require 'production_readiness\.media_transport_ready|media_transport_ready' "$SCRIPT" \
  'host decoded-frame E2E must prove the production media transport is active'
require 'production_readiness\.client_media_ready|production_readiness\.get\("client_media_ready"\)|"client_media_ready"' "$SCRIPT" \
  'host decoded-frame E2E must prove the receiver/browser reported client-presenting media'
require 'decoded_frames\.count' "$SCRIPT" \
  'host decoded-frame E2E must validate positive decoded frame count'
require 'decoded_frames\.width|decoded_width' "$SCRIPT" \
  'host decoded-frame E2E must validate decoded frame width'
require 'decoded_frames\.height|decoded_height' "$SCRIPT" \
  'host decoded-frame E2E must validate decoded frame height'
require 'rtp_packet_count' "$SCRIPT" \
  'host decoded-frame E2E must validate positive RTP packet count'
require 'selected_content_present' "$SCRIPT" \
  'host decoded-frame E2E must validate selected target content is present'
require 'unrelated_sentinel_present' "$SCRIPT" \
  'host decoded-frame E2E must validate unrelated sentinel exclusion'
require '\bsentinel_fixture\b' "$SCRIPT" \
  'host decoded-frame E2E must require a dual-target sentinel fixture'
require 'dual_target_non_leak' "$SCRIPT" \
  'host decoded-frame E2E must bind evidence to a dual-target non-leak proof'
require 'sentinel_fixture\.selected\.resource_ura|selected_fixture\.get\("resource_ura"\)' "$SCRIPT" \
  'host decoded-frame E2E must bind selected sentinel witness to the selected Resource URA'
require 'EASYNET_REMOTEAPP_SELECTED_SENTINEL_PID|selected_fixture\.get\("pid"\)' "$SCRIPT" \
  'host decoded-frame E2E must validate selected sentinel pid when host fixture provides it'
require 'EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PID|unrelated_fixture\.get\("pid"\)' "$SCRIPT" \
  'host decoded-frame E2E must validate unrelated sentinel pid when host fixture provides it'
require 'resolved_identity\.get\("pid"\).*selected_pid|app_window_set\.get\("primary_pid"\).*selected_pid' "$SCRIPT" \
  'host decoded-frame E2E must bind application sentinel pid to resolved identity or app window-set proof'
require 'sentinel_fixture\.unrelated\.placement|unrelated_fixture\.get\("placement"\)' "$SCRIPT" \
  'host decoded-frame E2E must require unrelated sentinel witness placement'
require 'full_display_leak_detected' "$SCRIPT" \
  'host decoded-frame E2E must validate no full-display leak'
require 'display_fallback_used' "$SCRIPT" \
  'host decoded-frame E2E must validate display fallback was not used'
require 'scope_widened' "$SCRIPT" \
  'host decoded-frame E2E must validate scope was not widened'
require '--probe-cmd|EASYNET_REMOTEAPP_FRAME_PROBE_CMD' "$SCRIPT" \
  'host decoded-frame E2E must allow explicit host probe injection'
require 'host-remoteapp-decoded-frame-probe\.sh|BUNDLED_PROBE' "$SCRIPT" \
  'host decoded-frame E2E must default to the bundled EasyNet host probe'
require 'host-remoteapp-sentinel-fixture\.sh|BUNDLED_SENTINEL_FIXTURE' "$SCRIPT" \
  'host decoded-frame E2E must expose the bundled host sentinel fixture'
require '--sentinel-fixture|EASYNET_REMOTEAPP_SENTINEL_FIXTURE' "$SCRIPT" \
  'host decoded-frame E2E must allow launching a real host sentinel fixture'
require '--sentinel-fixture-cmd|EASYNET_REMOTEAPP_SENTINEL_FIXTURE_CMD' "$SCRIPT" \
  'host decoded-frame E2E must allow explicit sentinel fixture injection'
require 'EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR' "$SCRIPT" \
  'host decoded-frame E2E must pass a fixture state directory to sentinel fixture commands'
require 'source "\$SENTINEL_FIXTURE_DIR/env\.sh"' "$SCRIPT" \
  'host decoded-frame E2E must source sentinel fixture env before running the probe'
require 'cleanup\.sh' "$SCRIPT" \
  'host decoded-frame E2E must run fixture cleanup after host probes'
require 'preflight_bundled_probe_runtime' "$SCRIPT" \
  'host decoded-frame E2E must preflight bundled EasyNet probe runtime before launching host fixtures'
require 'EASYNET_REMOTEAPP_CONTROL_DISCOVERY_JSON' "$SCRIPT" \
  'host decoded-frame E2E must allow explicit control discovery path for bundled probe preflight'
require 'daemon_identity' "$SCRIPT" \
  'host decoded-frame E2E bundled probe preflight must require daemon control discovery identity'
require '\$TIMESTAMP-\$TARGET_KIND-\$\$' "$SCRIPT" \
  'host decoded-frame E2E default report directory must isolate concurrent target-kind runs'
require 'os\.path\.isfile|decoded_frame_sample.*exist' "$SCRIPT" \
  'host decoded-frame E2E must validate decoded frame artifact exists'
require 'read_ppm_rgb|P6' "$SCRIPT" \
  'host decoded-frame E2E must independently parse decoded PPM artifacts'
require 'count_rgb_matches' "$SCRIPT" \
  'host decoded-frame E2E must independently scan decoded artifact pixels'
require 'selected_pixel_count' "$SCRIPT" \
  'host decoded-frame E2E must validate selected sentinel pixel count'
require 'unrelated_pixel_count' "$SCRIPT" \
  'host decoded-frame E2E must validate unrelated sentinel pixel count'
require 'artifacts\.binding_id|artifact binding_id' "$SCRIPT" \
  'host decoded-frame E2E must bind decoded frame artifact to target binding'
require 'target_binding\.binding_id' "$SCRIPT" \
  'host decoded-frame E2E must require a non-empty target binding id'
require 'target_binding\.binding_epoch' "$SCRIPT" \
  'host decoded-frame E2E must require a positive target binding epoch'
require 'target_binding\.subject_ura' "$SCRIPT" \
  'host decoded-frame E2E must require target binding subject URA evidence'
require 'target_binding\.target_identity_epoch' "$SCRIPT" \
  'host decoded-frame E2E must require target identity epoch evidence'
require 'target_binding\.target_geometry_revision' "$SCRIPT" \
  'host decoded-frame E2E must require target geometry revision evidence'
require 'target_binding\.media_source_epoch' "$SCRIPT" \
  'host decoded-frame E2E must require media source epoch evidence'
require 'target_binding\.consent_epoch' "$SCRIPT" \
  'host decoded-frame E2E must require consent epoch evidence'
require 'artifacts\.session_id|artifact session_id' "$SCRIPT" \
  'host decoded-frame E2E must bind decoded frame artifact to session id'

require 'ability refresh-remote-targets' "$PROBE" \
  'bundled host probe must invoke live target inventory through the EasyNet CLI'
require 'ability create-remote-desktop-session' "$PROBE" \
  'bundled host probe must invoke selected-resource remote_desktop.create_session through the EasyNet CLI'
require 'easynet-remoteapp-frame-receiver' "$PROBE" \
  'bundled host probe must default to the bundled WebRTC frame receiver'
require 'EASYNET_REMOTEAPP_FRAME_RECEIVER_CMD' "$PROBE" \
  'bundled host probe must allow a real WebRTC frame receiver override'
require 'EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB' "$PROBE" \
  'bundled host probe must require selected-target RGB sentinel configuration'
require 'EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB' "$PROBE" \
  'bundled host probe must require unrelated RGB sentinel configuration'
require 'EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL' "$PROBE" \
  'bundled host probe must require selected witness label configuration'
require 'EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL' "$PROBE" \
  'bundled host probe must require unrelated witness label configuration'
require '\bsentinel_fixture\b' "$PROBE" \
  'bundled host probe must publish canonical dual-target sentinel fixture evidence'
require 'EASYNET_REMOTEAPP_FRAME_ANALYSIS_JSON' "$PROBE" \
  'bundled host probe must consume decoded-frame analysis from the receiver'
require 'production_readiness = frame\.get\("production_readiness"\)' "$PROBE" \
  'bundled host probe must require post-negotiation production readiness from the frame receiver'
require 'production_readiness\.get\("client_media_ready"\)|"client_media_ready"' "$PROBE" \
  'bundled host probe must preserve client-presenting readiness evidence'
require '"production_media_ready": frame\.get\("production_media_ready"\)' "$PROBE" \
  'bundled host probe must carry post-negotiation production_media_ready into canonical evidence'
require 'verified Invocation\.subject|invocation\.get\("subject_ura"\)' "$PROBE" \
  'bundled host probe must validate verified Invocation.subject against the selected Resource URA'
require 'resolved_identity' "$PROBE" \
  'bundled host probe must preserve target resolved identity evidence'
require 'ambiguous target selection|TARGET_HINT|TARGET_RESOURCE_URA' "$PROBE" \
  'bundled host probe must fail closed on ambiguous picker target selection'
require 'EASYNET_REMOTEAPP_TARGET_PID' "$PROBE" \
  'bundled host probe must support native-pid target selection for application/window host fixtures'
require 'primary_pid' "$PROBE" \
  'bundled host probe must match native-pid target selection against primary_pid metadata'
require 'metadata\.get\("pid"\)' "$PROBE" \
  'bundled host probe must match native-pid target selection against pid metadata'

require 'swiftc' "$FIXTURE" \
  'bundled sentinel fixture must launch real native macOS windows through a compiled AppKit fixture'
require 'AppKit' "$FIXTURE" \
  'bundled sentinel fixture must use native AppKit windows instead of fake evidence'
require 'EASYNET_REMOTEAPP_TARGET_HINT' "$FIXTURE" \
  'bundled sentinel fixture must export a selected target hint for live inventory selection'
require 'EASYNET_REMOTEAPP_TARGET_PID' "$FIXTURE" \
  'bundled sentinel fixture must export a selected target pid for native identity selection'
require 'EASYNET_REMOTEAPP_SELECTED_SENTINEL_PID' "$FIXTURE" \
  'bundled sentinel fixture must export selected sentinel pid evidence'
require 'EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PID' "$FIXTURE" \
  'bundled sentinel fixture must export unrelated sentinel pid evidence'
require 'EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB' "$FIXTURE" \
  'bundled sentinel fixture must export selected RGB sentinel configuration'
require 'EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB' "$FIXTURE" \
  'bundled sentinel fixture must export unrelated RGB sentinel configuration'
require 'EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL' "$FIXTURE" \
  'bundled sentinel fixture must export selected witness label'
require 'EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL' "$FIXTURE" \
  'bundled sentinel fixture must export unrelated witness label'
require 'other_application' "$FIXTURE" \
  'bundled sentinel fixture must distinguish application non-leak placement from another application'
require 'manifest\.json' "$FIXTURE" \
  'bundled sentinel fixture must write a fixture manifest'
require 'cleanup\.sh' "$FIXTURE" \
  'bundled sentinel fixture must write an idempotent cleanup script'

require 'show-remote-desktop-session' "$RECEIVER" \
  'bundled frame receiver must read the latest post-decoded-frame session projection'
require 'report-remote-desktop-client-state' "$RECEIVER" \
  'bundled frame receiver must report decoded-frame client presentation before readiness projection'
require 'report_client_presenting\(config, signal\.transport_epoch\)' "$RECEIVER" \
  'bundled frame receiver must bind client-presenting report to the negotiated transport epoch'
require 'show_session_view\(config\)' "$RECEIVER" \
  'bundled frame receiver must source readiness from remote_desktop.show_session after decoded frames'
require '"production_media_ready": session_view' "$RECEIVER" \
  'bundled frame receiver must write production_media_ready from the latest session projection'
require '"production_readiness": session_view' "$RECEIVER" \
  'bundled frame receiver must write production_readiness from the latest session projection'
require '"client_media_ready": session_view' "$RECEIVER" \
  'bundled frame receiver must write explicit client-presenting readiness from the latest session projection'

bash "$SCRIPT" --self-test >/dev/null

printf 'check-remoteapp-e2e-acceptance-boundary: ok\n'
