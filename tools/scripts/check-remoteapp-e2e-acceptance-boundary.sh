#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
SCRIPT="$ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
PROBE="$ROOT/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
FIXTURE="$ROOT/tools/scripts/host-remoteapp-sentinel-fixture.sh"
CREATE_FAILCLOSED="$ROOT/tools/scripts/host-remoteapp-create-session-failclosed-e2e.sh"
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
[[ -f "$CREATE_FAILCLOSED" ]] || fail "missing host create_session fail-closed E2E harness"
[[ -f "$RECEIVER" ]] || fail "missing bundled host decoded-frame receiver"
[[ -f "$SPEC" ]] || fail "missing remoteapp targeted session SPEC"

require 'E2E-03 exact window session' "$SPEC" \
  'SPEC must retain exact window decoded-frame acceptance'
require 'E2E-04 exact application session' "$SPEC" \
  'SPEC must retain exact application decoded-frame acceptance'
require 'E2E-05 stale window fail-closed' "$SPEC" \
  'SPEC must retain stale window fail-closed acceptance'
require 'E2E-07 display fallback forbidden' "$SPEC" \
  'SPEC must retain display fallback decoded-frame acceptance'
require 'E2E-08 move/resize tracking' "$SPEC" \
  'SPEC must retain live move/resize acceptance'
require 'E2E-09 target loss vs transport failure' "$SPEC" \
  'SPEC must retain live target-loss acceptance'

require 'resource\.refresh_remote_targets' "$SCRIPT" \
  'host decoded-frame E2E must prove live refresh inventory was used'
require 'resource\.watch_remote_targets' "$SCRIPT" \
  'host decoded-frame E2E must allow streaming live inventory evidence'
require 'remote_desktop\.create_session' "$SCRIPT" \
  'host decoded-frame E2E must prove remote_desktop.create_session was invoked'
require 'Invocation\.subject|invocation\.subject_ura' "$SCRIPT" \
  'host decoded-frame E2E must validate selected resource URA as Invocation.subject'
require 'invocation_args = get\("invocation\.args"\)' "$SCRIPT" \
  'host decoded-frame E2E must inspect verified create_session invocation args'
require 'verified Invocation\.args must be reported as an object' "$SCRIPT" \
  'host decoded-frame E2E must reject evidence that omits verified Invocation.args'
require 'def contains_create_session_subject_arg\(value\):' "$SCRIPT" \
  'host decoded-frame E2E must recursively reject subject identity inside create_session args'
require 'remote_desktop\.create_session args must not contain subject, subject_ura, or resource_ura' "$SCRIPT" \
  'host decoded-frame E2E must prove create_session selected target is not passed through args'
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
require 'identity_display_id = resolved_identity\.get\("display_id"\)' "$SCRIPT" \
  'host decoded-frame E2E must read application resolved identity display id'
require 'application resolved_identity\.display_id must match app_window_set\.display_id' "$SCRIPT" \
  'host decoded-frame E2E must bind application identity to the display-scoped window set'
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
require '--lifecycle-scenario|EASYNET_REMOTEAPP_LIFECYCLE_SCENARIO' "$SCRIPT" \
  'host decoded-frame E2E must support explicit lifecycle acceptance scenarios'
require 'move-resize' "$SCRIPT" \
  'host lifecycle E2E must support move/resize evidence'
require 'target-loss' "$SCRIPT" \
  'host lifecycle E2E must support target-loss evidence'
require 'move-resize lifecycle evidence must include TARGET_MOVED' "$SCRIPT" \
  'host lifecycle E2E must validate TARGET_MOVED evidence'
require 'move-resize lifecycle evidence must include TARGET_RESIZED' "$SCRIPT" \
  'host lifecycle E2E must validate TARGET_RESIZED evidence'
require 'move-resize lifecycle evidence must show input transform consuming the latest geometry revision' "$SCRIPT" \
  'host lifecycle E2E must validate input transform geometry-revision coupling when pointer target is projected'
require 'move-resize lifecycle evidence without pointer target must keep pointer input disabled' "$SCRIPT" \
  'host lifecycle E2E must validate view-only move/resize keeps pointer input disabled'
require 'target-loss lifecycle evidence must include TARGET_LOST' "$SCRIPT" \
  'host lifecycle E2E must validate TARGET_LOST evidence'
require 'target-loss lifecycle evidence must include MEDIA_SOURCE_LOST' "$SCRIPT" \
  'host lifecycle E2E must validate MEDIA_SOURCE_LOST evidence'
require 'target-loss lifecycle evidence must not collapse target loss into TRANSPORT_FAILED' "$SCRIPT" \
  'host lifecycle E2E must validate target loss is not a transport failure'
require 'target-loss lifecycle evidence must leave the session suspended' "$SCRIPT" \
  'host lifecycle E2E must validate suspended state after target loss'
require 'target-loss lifecycle evidence must disable input' "$SCRIPT" \
  'host lifecycle E2E must validate input disablement after target loss'
require 'EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR' "$SCRIPT" \
  'host decoded-frame E2E must pass a fixture state directory to sentinel fixture commands'
require 'source "\$SENTINEL_FIXTURE_DIR/env\.sh"' "$SCRIPT" \
  'host decoded-frame E2E must source sentinel fixture env before running the probe'
require 'cleanup\.sh' "$SCRIPT" \
  'host decoded-frame E2E must run fixture cleanup after host probes'
require 'preflight_bundled_probe_runtime' "$SCRIPT" \
  'host decoded-frame E2E must preflight bundled EasyNet probe runtime before launching host fixtures'
require 'write_failure_report' "$SCRIPT" \
  'host decoded-frame E2E must write a structured failure report when enabled preflight cannot run'
require 'bundled_probe_preflight_failed' "$SCRIPT" \
  'host decoded-frame E2E failure report must identify bundled probe preflight failures'
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
require 'artifacts\.target_identity_epoch|artifact target_identity_epoch' "$SCRIPT" \
  'host decoded-frame E2E must bind decoded frame artifact to target identity epoch'
require 'artifacts\.target_geometry_revision|artifact target_geometry_revision' "$SCRIPT" \
  'host decoded-frame E2E must bind decoded frame artifact to target geometry revision'
require 'artifacts\.media_source_epoch|artifact media_source_epoch' "$SCRIPT" \
  'host decoded-frame E2E must bind decoded frame artifact to media source epoch'
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
require 'artifacts\.subject_ura|artifact subject_ura' "$SCRIPT" \
  'host decoded-frame E2E must bind decoded frame artifact to target subject'
require 'artifacts\.consent_epoch|artifact consent_epoch' "$SCRIPT" \
  'host decoded-frame E2E must bind decoded frame artifact to consent epoch'
require '"target_identity_epoch": config\.session_artifact\.target_identity_epoch' "$RECEIVER" \
  'bundled frame receiver must write target identity epoch into decoded artifacts'
require '"target_geometry_revision": config\.session_artifact\.target_geometry_revision' "$RECEIVER" \
  'bundled frame receiver must write target geometry revision into decoded artifacts'
require '"media_source_epoch": config\.session_artifact\.media_source_epoch' "$RECEIVER" \
  'bundled frame receiver must write media source epoch into decoded artifacts'
require '"subject_ura": config\.session_artifact\.subject_ura' "$RECEIVER" \
  'bundled frame receiver must write target subject into decoded artifacts'
require '"consent_epoch": config\.session_artifact\.consent_epoch' "$RECEIVER" \
  'bundled frame receiver must write consent epoch into decoded artifacts'
require 'session target_binding missing subject_ura' "$RECEIVER" \
  'bundled frame receiver must reject session artifacts without target subject'
require 'session target_binding missing positive target_identity_epoch' "$RECEIVER" \
  'bundled frame receiver must reject session artifacts without target identity epoch'
require 'session target_binding missing positive target_geometry_revision' "$RECEIVER" \
  'bundled frame receiver must reject session artifacts without target geometry revision'
require 'session target_binding missing positive media_source_epoch' "$RECEIVER" \
  'bundled frame receiver must reject session artifacts without media source epoch'
require 'session target_binding missing positive consent_epoch' "$RECEIVER" \
  'bundled frame receiver must reject session artifacts without consent epoch'

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
require '"args": invocation\.get\("args"\)' "$PROBE" \
  'bundled host probe must preserve create_session invocation args for no-subject-in-args evidence'
require 'verified remote_desktop\.create_session invocation metadata missing args object' "$PROBE" \
  'bundled host probe must fail closed when verified invocation metadata omits args'
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
require 'EASYNET_REMOTEAPP_LIFECYCLE_SCENARIO' "$PROBE" \
  'bundled host probe must consume lifecycle scenario selection'
require 'EASYNET_REMOTEAPP_SELECTED_CONTROL_SH' "$PROBE" \
  'bundled host probe must execute selected sentinel lifecycle controls'
require 'show-remote-desktop-session' "$PROBE" \
  'bundled host probe must record post-action daemon session projection for lifecycle evidence'
require 'evidence\["lifecycle"\] = lifecycle' "$PROBE" \
  'bundled host probe must publish canonical lifecycle evidence'

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
require 'selected-control\.sh' "$FIXTURE" \
  'bundled sentinel fixture must write a selected-window lifecycle control script'
require 'action == "move"' "$FIXTURE" \
  'bundled sentinel fixture must support a true move-only host lifecycle action'
require 'move_resize' "$FIXTURE" \
  'bundled sentinel fixture must support move/resize host lifecycle actions'
require 'window\.setFrame' "$FIXTURE" \
  'bundled sentinel fixture must move and resize through the native AppKit window'
require 'window\.close\(\)' "$FIXTURE" \
  'bundled sentinel fixture must support native selected-window close for target-loss E2E'
require 'EASYNET_REMOTEAPP_SELECTED_CONTROL_SH' "$FIXTURE" \
  'bundled sentinel fixture must export the selected lifecycle control helper'

require 'stale-window' "$CREATE_FAILCLOSED" \
  'host create_session fail-closed E2E must support stale-window scenario'
require 'resource\.refresh_remote_targets' "$CREATE_FAILCLOSED" \
  'host create_session fail-closed E2E must select from live target inventory before closing the window'
require 'EASYNET_REMOTEAPP_SELECTED_CONTROL_SH' "$CREATE_FAILCLOSED" \
  'host create_session fail-closed E2E must close the native selected window through the fixture control helper'
require '--session-id' "$CREATE_FAILCLOSED" \
  'host create_session fail-closed E2E must use a deterministic session id for absence probing'
require 'remote_desktop\.create_session' "$CREATE_FAILCLOSED" \
  'host create_session fail-closed E2E must invoke remote_desktop.create_session'
require 'target_not_found.*target_stale|target_stale.*target_not_found' "$CREATE_FAILCLOSED" \
  'host create_session fail-closed E2E must accept the SPEC stale-window failure reasons'
require 'refresh_targets' "$CREATE_FAILCLOSED" \
  'host create_session fail-closed E2E must require refresh_targets frontend action'
require 'remote_desktop\.show_session' "$CREATE_FAILCLOSED" \
  'host create_session fail-closed E2E must probe the deterministic session id after create failure'
require 'session_not_found' "$CREATE_FAILCLOSED" \
  'host create_session fail-closed E2E must prove no active session row was inserted'
require 'session_token_mismatch' "$CREATE_FAILCLOSED" \
  'host create_session fail-closed E2E must distinguish no-row absence from inserted-row token mismatch'
require 'create_session args must not contain subject, subject_ura, or resource_ura' "$CREATE_FAILCLOSED" \
  'host create_session fail-closed E2E must preserve selected Resource URA as Invocation.subject'

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
