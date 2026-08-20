#!/usr/bin/env bash
#
# Guard ability-list catalogue filtering against invocation-subject vocabulary.

set -euo pipefail

ROOT="${CHECK_ABILITY_CATALOGUE_SCOPE_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-ability-catalogue-scope-boundary: $*" >&2
    exit 1
}

ABILITIES_RS="src/cli/commands/abilities.rs"

[[ -f "$ABILITIES_RS" ]] || fail "missing $ABILITIES_RS"

grep -q 'enum AbilityCatalogueScope' "$ABILITIES_RS" \
    || fail "ability list must project public scope input through AbilityCatalogueScope"

grep -q 'fn from_cli_scope(scope_ura: Option<String>) -> anyhow::Result<Self>' "$ABILITIES_RS" \
    || fail "AbilityCatalogueScope must own CLI scope parsing"

grep -q 'fn into_parts(self) -> (Option<String>, Option<String>)' "$ABILITIES_RS" \
    || fail "AbilityCatalogueScope must project to canonical owner_ura/ability_ura parts"

grep -q 'let scope_ura = args' "$ABILITIES_RS" \
    || fail "ability list facade must name the public input as catalogue scope before projection"

grep -q 'AbilityCatalogueScope::from_cli_scope(scope_ura)?' "$ABILITIES_RS" \
    || fail "ability list query construction must use AbilityCatalogueScope"

grep -q 'Self::new(owner_ura, ability_ura)' "$ABILITIES_RS" \
    || fail "AbilityCatalogueQuery must receive only canonical catalogue fields"

bad="$(
    grep -nE 'classify_catalogue_subject_scope|subject_owner_ura|subject scope|Owner/subject scope|Public compatibility flag|AbilitySubjectScope|merge_owner_scope' \
        "$ABILITIES_RS" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "ability catalogue query still carries retired subject-scope vocabulary:
$bad"
fi

echo "check-ability-catalogue-scope-boundary: ok"
