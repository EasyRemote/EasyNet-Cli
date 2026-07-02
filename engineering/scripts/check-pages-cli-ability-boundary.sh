#!/usr/bin/env bash
#
# Guard the Pages CLI facade against scattered local ability-name construction.

set -euo pipefail

ROOT="${CHECK_PAGES_CLI_ABILITY_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-pages-cli-ability-boundary: $*" >&2
    exit 1
}

PAGES_RS="src/cli/pages.rs"
[[ -f "$PAGES_RS" ]] || fail "missing $PAGES_RS"

grep -q 'enum PagesAbilityVerb' "$PAGES_RS" \
    || fail "Pages CLI must model pages verbs with PagesAbilityVerb"

grep -q 'struct PagesAbility' "$PAGES_RS" \
    || fail "Pages CLI must route through the typed PagesAbility selector"

grep -q 'fn local_registry_ability(&self)' "$PAGES_RS" \
    || fail "PagesAbility must own local registry key projection"

invoke_body="$(
    perl -0ne 'print $1 if /fn invoke_pages_ability\(ability: &PagesAbility, args: Value\) -> anyhow::Result<Value> \{(.*?)\n\}/s' \
        "$PAGES_RS"
)"
[[ -n "$invoke_body" ]] \
    || fail "Pages CLI must route all ability calls through invoke_pages_ability"

grep -q 'let target = ability.local_target(&current_realm())?;' <<<"$invoke_body" \
    || fail "invoke_pages_ability must derive the typed LocalAbilityTarget from PagesAbility"

grep -q 'invoke_local_ability_target_with_subject_timeout(' <<<"$invoke_body" \
    || fail "invoke_pages_ability must use the typed local ability target helper"

grep -q '&target,' <<<"$invoke_body" \
    || fail "invoke_pages_ability must invoke the typed target, not a raw ability string"

bad="$(
    grep -nE 'format!\("\{user\}\.pages\.|let ability = format!\(|invoke_local_ability\(&ability,|"\.pages\.(publish|list|get|unpublish)"' "$PAGES_RS" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "Pages CLI still scatters local ability-name construction:
$bad"
fi

echo "check-pages-cli-ability-boundary: ok"
