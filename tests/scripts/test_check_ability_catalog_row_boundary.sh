#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-ability-catalog-row-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-ability-catalog-row-boundary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/cli/commands/groups"
    cp "$REPO_ROOT/src/cli/commands/ability_catalog_row.rs" "$sandbox/src/cli/commands/ability_catalog_row.rs"
    cp "$REPO_ROOT/src/cli/commands/groups/device.rs" "$sandbox/src/cli/commands/groups/device.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_ABILITY_CATALOG_ROW_BOUNDARY_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: strict catalogue row projection should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/pub\(crate\) fn from_value\(value: &Value\) -> anyhow::Result<Self>/pub(crate) fn from_value(value: \\&Value) -> Self/' "$SB/src/cli/commands/ability_catalog_row.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "non-Result projector should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/reject_retired_catalogue_fields\(value\)\?;//' "$SB/src/cli/commands/ability_catalog_row.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing retired-field rejection should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/projection_rejects_retired_ability_name_and_tool_name_fields/projection_ignores_legacy_aliases_as_label_fallback/' "$SB/src/cli/commands/ability_catalog_row.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired alias regression name should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/AbilityCatalogueRow::from_value\(a\)\?/AbilityCatalogueRow::from_value(a)/' "$SB/src/cli/commands/groups/device.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "non-propagating device projection should exit 1 (got $rc)"

echo "test_check_ability_catalog_row_boundary.sh: all cases passed"
