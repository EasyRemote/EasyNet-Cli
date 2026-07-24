#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-local-daemon-subject-owner-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-local-daemon-subject-owner-boundary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/support/platform" "$sandbox/src/daemon/identity" "$sandbox/tools/scripts"
    cp "$SCRIPT" "$sandbox/tools/scripts/check-local-daemon-subject-owner-boundary.sh"
    cp "$REPO_ROOT/src/support/platform/local_invoke.rs" "$sandbox/src/support/platform/local_invoke.rs"
    cp "$REPO_ROOT/src/support/platform/local_daemon_grpc.rs" "$sandbox/src/support/platform/local_daemon_grpc.rs"
    cp "$REPO_ROOT/src/daemon/identity/local_invocation.rs" "$sandbox/src/daemon/identity/local_invocation.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    (
        cd "$sandbox"
        CHECK_LOCAL_DAEMON_SUBJECT_OWNER_BOUNDARY_ROOT="$sandbox" \
            bash tools/scripts/check-local-daemon-subject-owner-boundary.sh
    )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: issuer-owned subject helper should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's#crate::daemon::identity::local_invocation::local_daemon_ura\(\)#crate::support::platform::local_daemon_grpc::local_daemon_identity_subject_ura()#' \
    "$SB/src/support/platform/local_invoke.rs"
rc=0
run_check "$SB" >/tmp/check-local-daemon-subject-owner-boundary.out 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "transport-sourced issuer should exit 1 (got $rc)"
grep -Fq "must source subject identity from daemon::identity::local_invocation" \
    /tmp/check-local-daemon-subject-owner-boundary.out \
    || fail "transport-sourced issuer failure should name identity owner"

SB="$(make_sandbox)"
cat >>"$SB/src/support/platform/local_daemon_grpc.rs" <<'RS'
pub(crate) fn local_daemon_identity_subject_ura() -> anyhow::Result<String> {
    local_daemon_identity_ura()
}
RS
rc=0
run_check "$SB" >/tmp/check-local-daemon-subject-owner-boundary.out 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "transport subject helper should exit 1 (got $rc)"
grep -Fq "transport must not expose subject-selection helpers" \
    /tmp/check-local-daemon-subject-owner-boundary.out \
    || fail "transport helper failure should name subject-selection ownership"

SB="$(make_sandbox)"
perl -0pi -e 's/pub\(crate\) fn local_daemon_ura\(\) -> anyhow::Result<String>/fn local_daemon_ura() -> anyhow::Result<String>/' \
    "$SB/src/daemon/identity/local_invocation.rs"
rc=0
run_check "$SB" >/tmp/check-local-daemon-subject-owner-boundary.out 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "private daemon identity owner should exit 1 (got $rc)"
grep -Fq "daemon::identity::local_invocation must own local daemon URA construction" \
    /tmp/check-local-daemon-subject-owner-boundary.out \
    || fail "identity owner failure should name daemon identity module"

echo "test_check_local_daemon_subject_owner_boundary.sh: all cases passed"
