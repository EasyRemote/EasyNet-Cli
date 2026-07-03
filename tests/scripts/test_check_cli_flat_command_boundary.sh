#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-cli-flat-command-boundary.sh.
#
# F-039 was reversed: `join` / `start` / `stop` are first-class top-level
# Quickstart commands, NOT retired aliases. The boundary guard therefore now
# asserts that BOTH the Quickstart shortcuts AND the layered groups exist.
# These contract tests verify that inverted intent: the guard passes on the
# real tree, and fails when any required command (Quickstart or layered) is
# removed.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-cli-flat-command-boundary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src" "$sandbox/scripts" "$sandbox/tests"
    cp -R "$REPO_ROOT/src/cli" "$sandbox/src/"
    cp -R "$REPO_ROOT/scripts" "$sandbox/"
    cp -R "$REPO_ROOT/tests" "$sandbox/"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_CLI_FLAT_COMMAND_BOUNDARY_ROOT="$sandbox" bash "$SCRIPT" )
}

# Happy path: the real tree has the Quickstart shortcuts + layered groups.
SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: full CLI surface should pass"; }
rm -rf "$SB"

# Removing the Quickstart `start` shortcut must fail (the guard now requires it).
SB="$(make_sandbox)"
perl -0pi -e 's#\n[[:space:]]*Start\(start::StartArgs\),##' "$SB/src/cli/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing Quickstart 'start' should exit 1 (got $rc)"

# Removing the Quickstart `join` shortcut must fail.
SB="$(make_sandbox)"
perl -0pi -e 's#\n[[:space:]]*Join\(join::JoinArgs\),##' "$SB/src/cli/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing Quickstart 'join' should exit 1 (got $rc)"

# Removing the layered `Device` group must fail (no behavioural drift: the
# layered home must stay so both spellings share the same impl).
SB="$(make_sandbox)"
perl -0pi -e 's#\n[[:space:]]*Device\(groups::device::DeviceArgs\),##' "$SB/src/cli/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing layered 'device' group should exit 1 (got $rc)"

echo "test_check_cli_flat_command_boundary.sh: all cases passed"
