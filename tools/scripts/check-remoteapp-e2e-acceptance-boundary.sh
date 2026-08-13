#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
SCRIPT="$ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
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
require 'transport\.kind.*webrtc|transport_kind.*webrtc|"webrtc"' "$SCRIPT" \
  'host decoded-frame E2E must validate WebRTC transport evidence'
require 'decoded_frames\.count' "$SCRIPT" \
  'host decoded-frame E2E must validate positive decoded frame count'
require 'selected_content_present' "$SCRIPT" \
  'host decoded-frame E2E must validate selected target content is present'
require 'unrelated_sentinel_present' "$SCRIPT" \
  'host decoded-frame E2E must validate unrelated sentinel exclusion'
require 'full_display_leak_detected' "$SCRIPT" \
  'host decoded-frame E2E must validate no full-display leak'
require 'display_fallback_used' "$SCRIPT" \
  'host decoded-frame E2E must validate display fallback was not used'
require 'scope_widened' "$SCRIPT" \
  'host decoded-frame E2E must validate scope was not widened'
require '--probe-cmd|EASYNET_REMOTEAPP_FRAME_PROBE_CMD' "$SCRIPT" \
  'host decoded-frame E2E must fail closed without a real host probe command'

bash "$SCRIPT" --self-test >/dev/null

printf 'check-remoteapp-e2e-acceptance-boundary: ok\n'
