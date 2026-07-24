#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-go-sdk-lifecycle-fixture-neutrality.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-go-sdk-lifecycle-fixture-neutrality.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/sdk/go" "$sandbox/tools/scripts"
    cp "$SCRIPT" "$sandbox/tools/scripts/check-go-sdk-lifecycle-fixture-neutrality.sh"
    cp "$REPO_ROOT/sdk/go/daemon_test.go" "$sandbox/sdk/go/daemon_test.go"
    cp "$REPO_ROOT/sdk/go/runtime_lifecycle.go" "$sandbox/sdk/go/runtime_lifecycle.go"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    (
        cd "$sandbox"
        CHECK_GO_SDK_LIFECYCLE_FIXTURE_NEUTRALITY_ROOT="$sandbox" \
            bash tools/scripts/check-go-sdk-lifecycle-fixture-neutrality.sh
    )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: generic lifecycle fixture should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
cat >>"$SB/sdk/go/daemon_test.go" <<'GO'
func (m *memoryDaemonTransport) CompanionStart(ctx context.Context, handleID string, packageID string, packageVersion string) ([]byte, error) {
	return []byte(`{"profile":"desktop_companion","kind":"desktop_companion_action_result"}`), nil
}
GO
rc=0
run_check "$SB" >/tmp/check-go-sdk-lifecycle-fixture-neutrality.out 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "companion method fixture should exit 1 (got $rc)"
grep -Fq "product companion lifecycle" /tmp/check-go-sdk-lifecycle-fixture-neutrality.out \
    || fail "companion method failure should name product companion lifecycle"

SB="$(make_sandbox)"
perl -0pi -e 's/readyDaemonStatus/companionStatusJSON/g' "$SB/sdk/go/daemon_test.go"
rc=0
run_check "$SB" >/tmp/check-go-sdk-lifecycle-fixture-neutrality.out 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "companion fixture helper name should exit 1 (got $rc)"
grep -Fq "product companion lifecycle" /tmp/check-go-sdk-lifecycle-fixture-neutrality.out \
    || fail "companion fixture helper failure should name product companion lifecycle"

SB="$(make_sandbox)"
perl -0pi -e 's/RuntimeLifecycleTransport interface/RuntimeLifecycleCarrier interface/' "$SB/sdk/go/runtime_lifecycle.go"
rc=0
run_check "$SB" >/tmp/check-go-sdk-lifecycle-fixture-neutrality.out 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing lifecycle interface should exit 1 (got $rc)"
grep -Fq "lifecycle transport interface is missing" /tmp/check-go-sdk-lifecycle-fixture-neutrality.out \
    || fail "missing lifecycle interface failure should name interface"

echo "test_check_go_sdk_lifecycle_fixture_neutrality.sh: all cases passed"
