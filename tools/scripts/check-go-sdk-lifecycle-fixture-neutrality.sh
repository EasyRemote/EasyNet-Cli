#!/usr/bin/env bash
#
# Guard Go SDK runtime lifecycle fixtures against product lifecycle leakage.

set -euo pipefail

ROOT="${CHECK_GO_SDK_LIFECYCLE_FIXTURE_NEUTRALITY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-go-sdk-lifecycle-fixture-neutrality: $*" >&2
    exit 1
}

DAEMON_TEST="sdk/go/daemon_test.go"
LIFECYCLE="sdk/go/runtime_lifecycle.go"
[[ -f "$DAEMON_TEST" ]] || fail "missing $DAEMON_TEST"
[[ -f "$LIFECYCLE" ]] || fail "missing $LIFECYCLE"

grep -Fq 'type RuntimeLifecycleTransport interface' "$LIFECYCLE" \
    || fail "Go SDK lifecycle transport interface is missing"

grep -Fq 'type memoryDaemonTransport struct' "$DAEMON_TEST" \
    || fail "Go SDK lifecycle fixture transport is missing"

for method in Discover Start Attach Status OpenRuntime Stop Detach; do
    grep -Eq "func \\(m \\*memoryDaemonTransport\\) ${method}\\(" "$DAEMON_TEST" \
        || fail "memoryDaemonTransport must implement canonical lifecycle method ${method}"
done

bad="$(
    rg -n 'Companion(List|Status|Enable|Disable|Start|Stop)|companion(Action|Status|Calls)|desktop_companion|easynet\\.desktop|EasyNet Menu Bar|launch_agent|boot_policy|stop_policy' \
        "$DAEMON_TEST" "$LIFECYCLE" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "Go SDK runtime lifecycle fixtures still expose product companion lifecycle:
$bad"
fi

echo "check-go-sdk-lifecycle-fixture-neutrality: ok"
