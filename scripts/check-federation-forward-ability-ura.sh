#!/usr/bin/env bash
#
# Guard remote federation invoke callers against ability+args-only entrypoints.

set -euo pipefail

ROOT="${CHECK_FEDERATION_FORWARD_ABILITY_URA_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-federation-forward-ability-ura: $*" >&2
    exit 1
}

[[ -f src/services/invocation_transport/federation_invoke.rs ]] || fail "missing src/services/invocation_transport/federation_invoke.rs"

bad="$(
    find src tests -name '*.rs' -print 2>/dev/null \
        | sort \
        | xargs grep -nE '(^|[^[:alnum:]_])invoke_via_federation_forward[[:space:]]*\(' 2>/dev/null \
        | grep -v 'invoke_via_federation_forward_ability_ura' || true
)"
if [[ -n "$bad" ]]; then
    fail "remote federation invoke still exposes ability+args-only wrapper/calls:
$bad"
fi

grep -q 'struct TargetOwnedAbilityUra' src/services/invocation_transport/federation_invoke.rs \
    || fail "missing explicit target-owned Ability URA value object"

if grep -q 'fn forward_ability_ura_for_target' src/services/invocation_transport/federation_invoke.rs; then
    fail "old target-owner Ability URA derivation helper must stay retired"
fi

grep -q 'fn invoke_via_federation_forward_ability_ura' src/services/invocation_transport/federation_invoke.rs \
    || fail "missing canonical Ability URA federation invoke helper"

echo "check-federation-forward-ability-ura: ok"
