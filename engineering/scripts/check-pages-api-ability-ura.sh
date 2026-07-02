#!/usr/bin/env bash
#
# Guard Pages API ability manifests to canonical Ability URA input.

set -euo pipefail

ROOT="${CHECK_PAGES_API_ABILITY_URA_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-pages-api-ability-ura: $*" >&2
    exit 1
}

API_RS="src/runtime/system_abilities/resources/pages/api.rs"
DOC_MD="docs/PAGES_AND_LLM_API.md"
[[ -f "$API_RS" ]] || fail "missing $API_RS"
[[ -f "$DOC_MD" ]] || fail "missing $DOC_MD"

grep -q '#\[serde(deny_unknown_fields)\]' "$API_RS" \
    || fail "ApiManifest must reject retired or misspelled manifest fields"

grep -q 'ability_ura: Option<String>' "$API_RS" \
    || fail "ApiManifest must expose ability_ura, not a bare local ability field"

grep -q 'AbilitySelector::parse(&ability_ura)' "$API_RS" \
    || fail "Pages API ability dispatch must parse canonical Ability URAs"

grep -q 'local_registry_ability()' "$API_RS" \
    || fail "Pages API ability dispatch must derive the local registry key from the Ability URA"

grep -q 'ability_ura = "easynet:///r/' "$DOC_MD" \
    || fail "Pages API docs must teach ability_ura with a canonical Ability URA"

bad="$(
    grep -nE 'ability: Option<String>|manifest\.ability\b|kind=ability requires `ability =|fully-qualified local ability' "$API_RS" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "Pages API still contains retired bare ability manifest plumbing:
$bad"
fi

doc_bad="$(
    grep -nE '^ability = "' "$DOC_MD" 2>/dev/null || true
)"
if [[ -n "$doc_bad" ]]; then
    fail "Pages API docs still advertise retired ability manifest field:
$doc_bad"
fi

echo "check-pages-api-ability-ura: ok"
