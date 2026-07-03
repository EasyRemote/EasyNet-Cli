#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

TARGET="src/daemon/execution/mission/dispatch.rs"

[[ -f "$TARGET" ]] || fail "missing $TARGET"

if rg -n 'send_to_agent_missing_mission_context|continuing in release|backwards compat|back-compat shim|active\.is_none|fallback.*mission context|EASYNET_AGENT_DEPTH"\.to_string\(\)' "$TARGET"; then
  fail "dispatch must hard-fail missing mission context instead of continuing in degraded mode"
fi

if rg -n '#\[cfg\((not\()?debug_assertions\)?\)\]' "$TARGET"; then
  fail "mission-context enforcement must not diverge between debug and release builds"
fi

if ! rg -n 'without a mission context' "$TARGET" >/dev/null; then
  fail "dispatch must expose an explicit missing mission-context error"
fi

if ! rg -n 'does not correspond to an existing' "$TARGET" >/dev/null; then
  fail "dispatch must reject forged mission ids whose run dir is absent"
fi

echo "check-dispatch-mission-context-boundary: ok"
