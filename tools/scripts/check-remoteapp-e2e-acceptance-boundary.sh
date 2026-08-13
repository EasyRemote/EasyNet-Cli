#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
SCRIPT="$ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
PROBE="$ROOT/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
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
require '"production_media_ready": frame\.get\("production_media_ready"\)' "$PROBE" \
  'bundled host probe must carry post-negotiation production_media_ready into canonical evidence'
require 'verified Invocation\.subject|invocation\.get\("subject_ura"\)' "$PROBE" \
  'bundled host probe must validate verified Invocation.subject against the selected Resource URA'
require 'resolved_identity' "$PROBE" \
  'bundled host probe must preserve target resolved identity evidence'
require 'ambiguous target selection|TARGET_HINT|TARGET_RESOURCE_URA' "$PROBE" \
  'bundled host probe must fail closed on ambiguous picker target selection'

require 'show-remote-desktop-session' "$RECEIVER" \
  'bundled frame receiver must read the latest post-decoded-frame session projection'
require 'show_session_view\(config\)' "$RECEIVER" \
  'bundled frame receiver must source readiness from remote_desktop.show_session after decoded frames'
require '"production_media_ready": session_view' "$RECEIVER" \
  'bundled frame receiver must write production_media_ready from the latest session projection'
require '"production_readiness": session_view' "$RECEIVER" \
  'bundled frame receiver must write production_readiness from the latest session projection'

bash "$SCRIPT" --self-test >/dev/null

printf 'check-remoteapp-e2e-acceptance-boundary: ok\n'
