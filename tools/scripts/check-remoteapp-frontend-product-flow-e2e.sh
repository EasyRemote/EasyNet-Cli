#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
FRONTEND_ROOT="${CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT:-$ROOT/../EasyNet/Frontend}"
HARNESS="$ROOT/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
PERMISSION_SUBJECT="$ROOT/tools/scripts/host-remoteapp-permission-subject-e2e.sh"
TARGET_FRESHNESS="$ROOT/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"
DECODED_FRAME="$ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
VIEW_ONLY_INPUT="$ROOT/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"
FRONTEND_UI_TEST="$FRONTEND_ROOT/src/components/easynet/DeviceMediaAccess.test.tsx"
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

[[ -f "$HARNESS" ]] || fail "missing frontend RemoteApp product-flow E2E harness"
[[ -x "$HARNESS" ]] || fail "frontend RemoteApp product-flow E2E harness must be executable"
[[ -f "$PERMISSION_SUBJECT" ]] || fail "missing host permission subject E2E harness"
[[ -f "$TARGET_FRESHNESS" ]] || fail "missing host target picker freshness E2E harness"
[[ -f "$DECODED_FRAME" ]] || fail "missing host decoded-frame E2E harness"
[[ -f "$VIEW_ONLY_INPUT" ]] || fail "missing host view-only input safety E2E harness"
[[ -f "$FRONTEND_UI_TEST" ]] || fail "missing frontend RemoteApp UI flow test"
[[ -f "$AUDIT" ]] || fail "missing RemoteApp product readiness audit"
[[ -f "$PLAN" ]] || fail "missing RemoteApp product closure evidence plan"

bash "$HARNESS" --self-test >/dev/null

require 'npx tsc --noEmit' "$HARNESS" \
  'product-flow harness must run frontend TypeScript checks'
require 'npm test -- src/components/easynet/DeviceMediaAccess\.test\.tsx' "$HARNESS" \
  'product-flow harness must run DeviceMediaAccess RemoteApp UI flow coverage'
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
require 'does not claim product completion' "$HARNESS" \
  'product-flow harness must explicitly avoid product-complete claims'

require 'runs the remote desktop UI flow from target picker through session end' "$FRONTEND_UI_TEST" \
  'frontend component test must cover picker-to-session-end user flow'
require 'watch_events' "$FRONTEND_UI_TEST" \
  'frontend UI flow test must prove watch_events is part of the session lifecycle'

require 'frontend-remoteapp-product-flow-e2e\.sh' "$AUDIT" \
  'product readiness audit must mention the product-flow E2E harness'
require 'runnable product-flow harness entrypoint' "$AUDIT" \
  'product readiness audit must classify the harness as an entrypoint, not proof of completion'
require 'Browser/Tauri E2E for full user flow with real backend/runtime' "$AUDIT" \
  'product readiness audit must retain real Browser/Tauri full-flow evidence as still required'
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

printf 'check-remoteapp-frontend-product-flow-e2e: ok\n'
