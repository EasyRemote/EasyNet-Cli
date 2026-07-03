#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-invoke-ability-ura-input.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-invoke-ability-ura-input.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/daemon/ability/builtins/agents"
    cp "$REPO_ROOT/src/daemon/ability/builtins/agents/invoke.rs" "$sandbox/src/daemon/ability/builtins/agents/invoke.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_INVOKE_ABILITY_URA_INPUT_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: ability_ura input contract should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/get\("ability_ura"\)/get("ability")/' "$SB/src/daemon/ability/builtins/agents/invoke.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired ability parser field should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/"required": \["ability_ura"\]/"required": ["ability"]/' "$SB/src/daemon/ability/builtins/agents/invoke.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired ability schema requirement should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/"ability_ura": \{/"target": { "type": "string" },\n            "ability_ura": {/' "$SB/src/daemon/ability/builtins/agents/invoke.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired target schema property should exit 1 (got $rc)"

echo "test_check_invoke_ability_ura_input.sh: all cases passed"
