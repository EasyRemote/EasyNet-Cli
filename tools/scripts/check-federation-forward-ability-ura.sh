#!/usr/bin/env bash
#
# Guard remote federation invoke callers against ability+args-only entrypoints.

set -euo pipefail

ROOT="${CHECK_FEDERATION_FORWARD_ABILITY_URA_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-federation-forward-ability-ura: $*" >&2
    exit 1
}

[[ -f src/daemon/invocation/routing/federation_invoke.rs ]] || fail "missing src/daemon/invocation/routing/federation_invoke.rs"

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

bad_ability_ura_calls="$(
    find src tests -name '*.rs' -print 2>/dev/null \
        | grep -v 'src/daemon/invocation/routing/federation_invoke.rs' \
        | sort \
        | xargs grep -nE 'invoke_via_federation_forward_ability_ura(_with_timeout)?[[:space:]]*\(' 2>/dev/null || true
)"
if [[ -n "$bad_ability_ura_calls" ]]; then
    fail "remote federation invoke callers must pass RemoteAbilityInvocationTarget:
$bad_ability_ura_calls"
fi

grep -q 'struct RemoteAbilityInvocationTarget' src/daemon/invocation/routing/federation_invoke.rs \
    || fail "missing explicit remote ability invocation target value object"

if grep -q 'TargetOwnedAbilityUra' src/daemon/invocation/routing/federation_invoke.rs; then
    fail "old target-owned Ability URA compatibility type must stay retired"
fi

grep -q 'fn invoke_via_federation_forward_target' src/daemon/invocation/routing/federation_invoke.rs \
    || fail "missing target-object federation invoke helper"

echo "check-federation-forward-ability-ura: ok"
