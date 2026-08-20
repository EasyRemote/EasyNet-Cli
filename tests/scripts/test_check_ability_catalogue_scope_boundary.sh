#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-ability-catalogue-scope-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-ability-catalogue-scope-boundary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/cli/commands"
    cp "$REPO_ROOT/src/cli/commands/abilities.rs" "$sandbox/src/cli/commands/abilities.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_ABILITY_CATALOGUE_SCOPE_BOUNDARY_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: ability catalogue scope boundary should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/enum AbilityCatalogueScope/enum AbilitySubjectScope/' "$SB/src/cli/commands/abilities.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired AbilitySubjectScope should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/catalogue_owner_ura: Option<String>/subject_owner_ura: Option<String>/' "$SB/src/cli/commands/abilities.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "subject_owner_ura regression should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's#Catalogue scope is sent#Owner/subject scope is sent#' "$SB/src/cli/commands/abilities.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "owner/subject scope wording should exit 1 (got $rc)"

echo "test_check_ability_catalogue_scope_boundary.sh: all cases passed"
