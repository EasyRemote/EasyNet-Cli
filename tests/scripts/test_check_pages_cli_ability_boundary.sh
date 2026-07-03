#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-pages-cli-ability-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-pages-cli-ability-boundary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/cli/commands"
    cp "$REPO_ROOT/src/cli/commands/pages.rs" "$sandbox/src/cli/commands/pages.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_PAGES_CLI_ABILITY_BOUNDARY_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: Pages CLI ability boundary should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/enum PagesAbilityVerb/enum PageVerb/' "$SB/src/cli/commands/pages.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing PagesAbilityVerb should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/let ability = PagesAbility::for_user\(&user, PagesAbilityVerb::Publish\)\?;/let ability = format!("{user}.pages.publish");/' "$SB/src/cli/commands/pages.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "scattered format construction should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/&target,\n        args,/&ability.local_registry_ability(),\n        args,/' "$SB/src/cli/commands/pages.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "direct ability variable invoke should exit 1 (got $rc)"

echo "test_check_pages_cli_ability_boundary.sh: all cases passed"
