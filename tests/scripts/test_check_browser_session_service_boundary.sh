#!/usr/bin/env bash
#
# Contract tests for scripts/check-browser-session-service-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/check-browser-session-service-boundary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/runtime/agents"
    cp "$REPO_ROOT/src/runtime/agents/browser_session_ability.rs" "$sandbox/src/runtime/agents/browser_session_ability.rs"
    cp "$REPO_ROOT/src/runtime/agents/real_invoke_tests.rs" "$sandbox/src/runtime/agents/real_invoke_tests.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_BROWSER_SESSION_SERVICE_BOUNDARY_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: browser service boundary should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/struct BrowserSessionService/struct RetiredBrowserSessionService/' "$SB/src/runtime/agents/browser_session_ability.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing BrowserSessionService should exit 1 (got $rc)"

SB="$(make_sandbox)"
{
    echo 'use std::sync::OnceLock;'
    echo 'fn store() {}'
} >> "$SB/src/runtime/agents/browser_session_ability.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "global store compatibility path should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/let service = Arc::new\(BrowserSessionService::default\(\)\);/let service = BrowserSessionService::default();/' "$SB/src/runtime/agents/browser_session_ability.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "registry without shared service Arc should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/service\.capture_viewport/capture_viewport_handler/' "$SB/src/runtime/agents/browser_session_ability.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "free capture handler regression should exit 1 (got $rc)"

echo "test_check_browser_session_service_boundary.sh: all cases passed"
