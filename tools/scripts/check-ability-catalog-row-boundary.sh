#!/usr/bin/env bash
#
# Guard the CLI ability catalogue projection against retired alias rows.

set -euo pipefail

ROOT="${CHECK_ABILITY_CATALOG_ROW_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-ability-catalog-row-boundary: $*" >&2
    exit 1
}

ROW_RS="src/cli/commands/ability_catalog_row.rs"
DEVICE_RS="src/cli/commands/groups/device.rs"
[[ -f "$ROW_RS" ]] || fail "missing $ROW_RS"
[[ -f "$DEVICE_RS" ]] || fail "missing $DEVICE_RS"

grep -Fq 'pub(crate) fn from_value(value: &Value) -> anyhow::Result<Self>' "$ROW_RS" \
    || fail "ability catalogue row projector must return Result and fail closed"

grep -Fq 'const RETIRED_CATALOGUE_FIELDS: &[&str] = &["ability_name", "tool_name"]' "$ROW_RS" \
    || fail "retired catalogue fields must stay centrally declared"

grep -Fq 'reject_retired_catalogue_fields(value)?' "$ROW_RS" \
    || fail "projector must reject retired catalogue fields before projection"

grep -Fq 'projection_rejects_retired_ability_name_and_tool_name_fields' "$ROW_RS" \
    || fail "missing regression test for retired ability_name/tool_name aliases"

grep -Fq 'AbilityCatalogueRow::from_value(a)?' "$DEVICE_RS" \
    || fail "device catalogue rendering must propagate strict projection errors"

bad="$(
    grep -nE 'ability_name.*intentionally ignored|tool_name.*intentionally ignored|projection_ignores_legacy_aliases|or_else\(\|\s*string_field\(value, "(ability_name|tool_name)"\)|unwrap_or_else\(\|\s*string_field\(value, "(ability_name|tool_name)"\)' \
        "$ROW_RS" "$DEVICE_RS" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "ability catalogue row still tolerates retired aliases:
$bad"
fi

echo "check-ability-catalog-row-boundary: ok"
