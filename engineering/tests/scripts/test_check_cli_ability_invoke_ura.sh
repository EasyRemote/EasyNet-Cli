#!/usr/bin/env bash
#
# Contract tests for scripts/check-cli-ability-invoke-ura.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/engineering/scripts/check-cli-ability-invoke-ura.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/cli/groups" "$sandbox/engineering/scripts"
    cp "$REPO_ROOT/src/cli/invoke.rs" "$sandbox/src/cli/invoke.rs"
    cp "$REPO_ROOT/src/cli/groups/ability.rs" "$sandbox/src/cli/groups/ability.rs"
    cp "$REPO_ROOT/engineering/scripts/control-smoke.sh" "$sandbox/engineering/scripts/control-smoke.sh"
    cp "$REPO_ROOT/engineering/scripts/chat-as-ability-smoke.sh" "$sandbox/engineering/scripts/chat-as-ability-smoke.sh"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_CLI_ABILITY_INVOKE_URA_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: CLI Ability URA input should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/pub ability_ura: String/pub ability: String/' "$SB/src/cli/invoke.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired ability field should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/invoke <ability-ura>/invoke <ability>/' "$SB/src/cli/groups/ability.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired help placeholder should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/"\$ABILITY_URA"/observe.health/' "$SB/engineering/scripts/control-smoke.sh"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired smoke bare selector should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/ABILITY_URA/ABILITY/g' "$SB/engineering/scripts/chat-as-ability-smoke.sh"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired chat smoke selector variable should exit 1 (got $rc)"

echo "test_check_cli_ability_invoke_ura.sh: all cases passed"
