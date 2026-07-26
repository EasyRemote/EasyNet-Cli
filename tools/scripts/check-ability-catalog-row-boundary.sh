#!/usr/bin/env bash
#
# Guard the CLI ability catalogue projection against non-canonical alias rows.

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

grep -Fq '#[serde(deny_unknown_fields)]' "$ROW_RS" \
    || fail "ability catalogue row projector must use an exact schema DTO"

grep -Fq 'struct AbilityCatalogueRowWire' "$ROW_RS" \
    || fail "ability catalogue row projector must parse through a presentation-local wire DTO"

grep -Fq 'serde_json::from_value(value.clone())' "$ROW_RS" \
    || fail "ability catalogue row projector must enter through serde schema validation"

grep -Fq 'projection_rejects_non_canonical_catalogue_alias_fields' "$ROW_RS" \
    || fail "missing regression test for non-canonical ability_name/tool_name aliases"

grep -Fq 'AbilityCatalogueRow::from_value(a)?' "$DEVICE_RS" \
    || fail "device catalogue rendering must propagate strict projection errors"

bad_retired_branch="$(
    grep -nE 'RETIRED_CATALOGUE_FIELDS|reject_retired_catalogue_fields|ability catalogue row contains retired field|retired field\\(s\\)' \
        "$ROW_RS" "$DEVICE_RS" 2>/dev/null || true
)"
if [[ -n "$bad_retired_branch" ]]; then
    fail "ability catalogue row still has retired-field branch:
$bad_retired_branch"
fi

bad="$(
    grep -nE 'ability_name.*intentionally ignored|tool_name.*intentionally ignored|projection_ignores_legacy_aliases|or_else\(\|\s*string_field\(value, "(ability_name|tool_name)"\)|unwrap_or_else\(\|\s*string_field\(value, "(ability_name|tool_name)"\)' \
        "$ROW_RS" "$DEVICE_RS" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "ability catalogue row still tolerates retired aliases:
$bad"
fi

echo "check-ability-catalog-row-boundary: ok"
