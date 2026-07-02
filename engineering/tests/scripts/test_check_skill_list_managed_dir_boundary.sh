#!/usr/bin/env bash
#
# Contract tests for scripts/check-skill-list-managed-dir-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/engineering/scripts/check-skill-list-managed-dir-boundary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/runtime/system_abilities/resources/skills"
    cp "$REPO_ROOT/src/runtime/system_abilities/resources/skills/list.rs" "$sandbox/src/runtime/system_abilities/resources/skills/list.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_SKILL_LIST_MANAGED_DIR_BOUNDARY_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: skill list managed-dir boundary should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/root\.join\("\.claude"\)\.join\("skills"\)/root.join("skills")/' "$SB/src/runtime/system_abilities/resources/skills/list.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "claude-code root-level skills path should exit 1 (got $rc)"

SB="$(make_sandbox)"
echo '// legacy <root>/skills compatibility scan' >> "$SB/src/runtime/system_abilities/resources/skills/list.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "legacy scan language should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/fn managed_skill_dir_for_agent_type/fn retired_managed_skill_dir_for_agent_type/' "$SB/src/runtime/system_abilities/resources/skills/list.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing centralized directory selector should exit 1 (got $rc)"

echo "test_check_skill_list_managed_dir_boundary.sh: all cases passed"
