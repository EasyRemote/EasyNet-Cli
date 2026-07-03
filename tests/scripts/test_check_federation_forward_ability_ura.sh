#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-federation-forward-ability-ura.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-federation-forward-ability-ura.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/daemon/invocation/routing" "$sandbox/tests"
    cp -R "$REPO_ROOT/src/support" "$sandbox/src/support"
    cp "$REPO_ROOT/src/daemon/invocation/routing/federation_invoke.rs" "$sandbox/src/daemon/invocation/routing/federation_invoke.rs"
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
cat >>"$SB/src/daemon/invocation/routing/federation_invoke.rs" <<'RS'

pub fn invoke_via_federation_forward() {}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "ability+args wrapper definition should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/cli"
cat >"$SB/src/cli/probe.rs" <<'RS'
pub fn probe() {
    crate::daemon::invocation::routing::federation_invoke::invoke_via_federation_forward();
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "ability+args wrapper call should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >>"$SB/src/daemon/invocation/routing/federation_invoke.rs" <<'RS'

fn forward_ability_ura_for_target() {}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "0" ]] || fail "retired helper-name sentinel should no longer be policy (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/cli"
cat >"$SB/src/cli/probe.rs" <<'RS'
pub fn probe() {
    crate::daemon::invocation::routing::federation_invoke::invoke_via_federation_forward_ability_ura(
        "easynet:///r/acme/ability/device.dev.fs.read",
        serde_json::json!({}),
        "easynet:///r/acme/device/dev",
        None,
    );
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "callers bypassing RemoteAbilityInvocationTarget should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >>"$SB/src/daemon/invocation/routing/federation_invoke.rs" <<'RS'

type TargetOwnedAbilityUra = RemoteAbilityInvocationTarget;
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "old compatibility type should exit 1 (got $rc)"

echo "test_check_federation_forward_ability_ura.sh: all cases passed"
