#!/usr/bin/env bash
#
# Contract tests for scripts/check-federation-forward-ability-ura.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/check-federation-forward-ability-ura.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/services/invocation_transport" "$sandbox/tests"
    cp -R "$REPO_ROOT/src/support" "$sandbox/src/support"
    cp "$REPO_ROOT/src/services/invocation_transport/federation_invoke.rs" "$sandbox/src/services/invocation_transport/federation_invoke.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_FEDERATION_FORWARD_ABILITY_URA_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: Ability URA boundary should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
cat >>"$SB/src/services/invocation_transport/federation_invoke.rs" <<'RS'

pub fn invoke_via_federation_forward() {}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "ability+args wrapper definition should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/facade/cli"
cat >"$SB/src/facade/cli/probe.rs" <<'RS'
pub fn probe() {
    crate::services::invocation_transport::federation_invoke::invoke_via_federation_forward();
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "ability+args wrapper call should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >>"$SB/src/services/invocation_transport/federation_invoke.rs" <<'RS'

fn forward_ability_ura_for_target() {}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "old target-owner helper should exit 1 (got $rc)"

echo "test_check_federation_forward_ability_ura.sh: all cases passed"
