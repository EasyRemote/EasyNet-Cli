#!/usr/bin/env bash
#
# Guard the human CLI `easynet ability invoke` front door to Ability URA input.

set -euo pipefail

ROOT="${CHECK_CLI_ABILITY_INVOKE_URA_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-cli-ability-invoke-ura: $*" >&2
    exit 1
}

INVOKE_RS="src/cli/commands/invoke.rs"
INVOCATION_TUPLE_RS="src/cli/commands/invocation_tuple.rs"
GROUP_RS="src/cli/commands/groups/ability.rs"
CONTROL_SMOKE="tools/scripts/control-smoke.sh"
CHAT_SMOKE="tools/scripts/chat-as-ability-smoke.sh"
[[ -f "$INVOKE_RS" ]] || fail "missing $INVOKE_RS"
[[ -f "$INVOCATION_TUPLE_RS" ]] || fail "missing $INVOCATION_TUPLE_RS"
[[ -f "$GROUP_RS" ]] || fail "missing $GROUP_RS"
[[ -f "$CONTROL_SMOKE" ]] || fail "missing $CONTROL_SMOKE"
[[ -f "$CHAT_SMOKE" ]] || fail "missing $CHAT_SMOKE"

grep -q 'pub ability_ura: String' "$INVOKE_RS" \
    || fail "CLI invoke args must name the public selector ability_ura"

grep -q 'AbilityInvocationRef::parse(&invoke_args.ability_ura)' "$INVOKE_RS" \
    || fail "CLI invoke must parse the public selector through the Ability URA boundary object"

grep -q 'AbilitySelector::parse(raw)' "$INVOCATION_TUPLE_RS" \
    || fail "plain CLI invoke input must still parse as an Ability URA"

grep -q 'AbilitySelector::parse(&ability_ura)' "$INVOCATION_TUPLE_RS" \
    || fail "descriptor-ref CLI invoke input must parse the embedded Ability URA"

grep -q 'local_registry_ability()' "$INVOKE_RS" \
    || fail "local CLI invoke must derive daemon registry key from Ability URA"

grep -q 'ABILITY_URA=' "$CHAT_SMOKE" \
    || fail "chat smoke must name the public selector ABILITY_URA"

bad="$(
    grep -nE 'pub ability: String|invoke_args\.ability\b|ability invoke <ability>|ability invoke <name>|invoke <ability>|Ability \(tool\) name|ability invoke observe\.health([[:space:]]|$)|ability invoke "\$ABILITY"|\bABILITY=' \
        "$INVOKE_RS" "$GROUP_RS" "$CONTROL_SMOKE" "$CHAT_SMOKE" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "CLI invoke still advertises or consumes retired bare ability input:
$bad"
fi

echo "check-cli-ability-invoke-ura: ok"
