#!/usr/bin/env bash
#
# Guard browser.* session state against process-global stores.

set -euo pipefail

ROOT="${CHECK_BROWSER_SESSION_SERVICE_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-browser-session-service-boundary: $*" >&2
    exit 1
}

BROWSER_RS="src/daemon/ability/builtins/device_control/browser.rs"
REAL_TESTS_RS="src/daemon/ability/builtins/real_invoke_tests.rs"

for file in "$BROWSER_RS" "$REAL_TESTS_RS"; do
    [[ -f "$file" ]] || fail "missing $file"
done

grep -q 'struct BrowserSessionService' "$BROWSER_RS" \
    || fail "browser session state must be owned by BrowserSessionService"

register_body="$(
    perl -0ne 'print $1 if /pub fn register\(reg: &mut AxonAbilityCatalog\) \{(.*?)\n\}/s' \
        "$BROWSER_RS"
)"
[[ -n "$register_body" ]] \
    || fail "browser registry must expose pub fn register(reg: &mut AxonAbilityCatalog)"

grep -q 'let service = Arc::new(BrowserSessionService::default())' <<<"$register_body" \
    || fail "browser registry must register closures over one explicit service instance"

grep -q 'service.capture_viewport' "$BROWSER_RS" \
    || fail "browser capture tests must exercise the service method directly"

grep -q 'same dispatcher' "$REAL_TESTS_RS" \
    || fail "real-invoke browser tests must keep session lifecycle in one dispatcher"

bad="$(
    grep -nE 'OnceLock|lazy_static|static STORE|fn store\(\)|process-global|open_session_handler|send_input_handler|capture_viewport_handler|close_session_handler|allow\(dead_code\)' \
        "$BROWSER_RS" 2>/dev/null || true
    grep -nE 'OnceLock|lazy_static|static STORE|fn store\(\)|process-global|open_session_handler|send_input_handler|capture_viewport_handler|close_session_handler' \
        "$REAL_TESTS_RS" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "browser session surface still carries retired global-state plumbing:
$bad"
fi

echo "check-browser-session-service-boundary: ok"
