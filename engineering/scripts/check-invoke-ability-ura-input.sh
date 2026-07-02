#!/usr/bin/env bash
#
# Guard <agent>.invoke against retired target/ability input fields.

set -euo pipefail

ROOT="${CHECK_INVOKE_ABILITY_URA_INPUT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-invoke-ability-ura-input: $*" >&2
    exit 1
}

INVOKE_RS="src/runtime/system_abilities/agents/invoke.rs"
[[ -f "$INVOKE_RS" ]] || fail "missing $INVOKE_RS"

grep -q 'get("ability_ura")' "$INVOKE_RS" \
    || fail "invoke parser must read the canonical ability_ura input"

grep -q '"required": \["ability_ura"\]' "$INVOKE_RS" \
    || fail "invoke schema must require ability_ura"

grep -q '"ability_ura": {' "$INVOKE_RS" \
    || fail "invoke schema must advertise ability_ura"

bad="$(
    grep -nE 'get\("ability"\)|get\("target"\)|"required": \["ability"\]|"target": \{|target\?: string|ability: string' "$INVOKE_RS" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "invoke still exposes retired input contract:
$bad"
fi

grep -q 'parse_rejects_retired_target_field' "$INVOKE_RS" \
    || fail "missing regression test for retired target input"

grep -q 'parse_rejects_retired_ability_field' "$INVOKE_RS" \
    || fail "missing regression test for retired ability input"

echo "check-invoke-ability-ura-input: ok"
