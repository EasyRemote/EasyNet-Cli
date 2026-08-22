#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
SPEC="$ROOT/docs/design/remoteapp-targeted-session-spec.md"
AUDIT="$ROOT/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
PLAN="$ROOT/pr/20260822-remoteapp-product-closure/02-evidence-audit.md"

fail() {
  printf 'check-remoteapp-product-closure-audit: %s\n' "$1" >&2
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

[[ -f "$SPEC" ]] || fail "missing RemoteApp targeted-session SPEC"
[[ -f "$AUDIT" ]] || fail "missing RemoteApp product readiness audit"
[[ -f "$PLAN" ]] || fail "missing RemoteApp product closure evidence plan"

reject 'full acceptance verified' "$SPEC" \
  'targeted-session SPEC must not claim full product acceptance'
require 'full RemoteApp product closure incomplete' "$SPEC" \
  'targeted-session SPEC must state that full RemoteApp product closure is incomplete'
require 'Interactive app/window input must remain view-only' "$SPEC" \
  'SPEC must retain the view-only input limitation until input execution is proven'
require 'Clipboard and file-drop frame types exist in the input model but are not implemented' "$SPEC" \
  'SPEC must retain unsupported clipboard/file-drop boundary'
require 'MultiAppSurface' "$SPEC" \
  'SPEC must retain multi-display application capture limitation'

require 'Status: product closure incomplete' "$AUDIT" \
  'audit must explicitly mark product closure incomplete'
require 'Passing the current boundary gates' "$AUDIT" \
  'audit must name current boundary gates'
require 'does not mean RemoteApp is product-complete' "$AUDIT" \
  'audit must distinguish boundary gates from product completion'
require 'Application/window selection and stable capture across macOS/Windows/Linux' "$AUDIT" \
  'audit must cover cross-platform application/window/display capture'
require 'Mouse/keyboard input injection is controllable' "$AUDIT" \
  'audit must cover product input injection'
require 'Audio/video codec, frame rate, bitrate adaptation' "$AUDIT" \
  'audit must cover media codec/adaptation'
require 'Multi-window/multi-application independent tracking' "$AUDIT" \
  'audit must cover multi-window/application tracking as execution effect'
require 'Disconnect/reconnect, session resume, consent revoke, cancel, timeout' "$AUDIT" \
  'audit must cover recovery and lifecycle closure'
require 'NAT/relay/WebRTC/direct fallback network paths' "$AUDIT" \
  'audit must cover real network paths'
require 'Frontend UI can discover, authorize, start, display, control, and end session' "$AUDIT" \
  'audit must cover frontend full lifecycle'
require 'Cross-device E2E smoke/regression exists beyond local provider boundary' "$AUDIT" \
  'audit must cover cross-device proof'
require 'source-contract checker, unit test, local provider' "$AUDIT" \
  'audit must name weak evidence classes'
require 'benchmark, or SPEC statement is insufficient' "$AUDIT" \
  'audit must define authoritative product evidence strictly'
require 'RemoteApp interactive desktop product: incomplete' "$AUDIT" \
  'audit must preserve the current product status'

require 'Full interactive RemoteApp product: incomplete' "$PLAN" \
  'plan evidence audit must keep the goal open'
require 'Cross-platform capture implementation/evidence for Windows and Linux' "$PLAN" \
  'plan evidence audit must list missing Windows/Linux evidence'
require 'Frontend full lifecycle E2E' "$PLAN" \
  'plan evidence audit must list frontend full lifecycle E2E as missing'

printf 'check-remoteapp-product-closure-audit: ok\n'
