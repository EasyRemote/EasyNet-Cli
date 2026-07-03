#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-pages-api-ability-ura.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-pages-api-ability-ura.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/daemon/ability/builtins/resources/pages" "$sandbox/docs"
    cp "$REPO_ROOT/src/daemon/ability/builtins/resources/pages/api.rs" "$sandbox/src/daemon/ability/builtins/resources/pages/api.rs"
    cp "$REPO_ROOT/docs/PAGES_AND_LLM_API.md" "$sandbox/docs/PAGES_AND_LLM_API.md"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_PAGES_API_ABILITY_URA_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: Pages API Ability URA contract should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/#\[serde\(deny_unknown_fields\)\]\n//' "$SB/src/daemon/ability/builtins/resources/pages/api.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing deny_unknown_fields should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/ability_ura: Option<String>/ability: Option<String>/' "$SB/src/daemon/ability/builtins/resources/pages/api.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired ability field should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/AbilitySelector::parse\(&ability_ura\)/crate::ura::parse_ura(&ability_ura)/' "$SB/src/daemon/ability/builtins/resources/pages/api.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "bypassing AbilitySelector should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/^ability_ura = "easynet:\/\/\/r\/[^"]+"/ability = "web-builder.todo_add_task"/m' "$SB/docs/PAGES_AND_LLM_API.md"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired doc manifest field should exit 1 (got $rc)"

echo "test_check_pages_api_ability_ura.sh: all cases passed"
