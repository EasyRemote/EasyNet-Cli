#!/usr/bin/env bash
#
# Guard <agent>.discover scope literals against retired compatibility aliases.

set -euo pipefail

ROOT="${CHECK_DISCOVER_SCOPE_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-discover-scope-boundary: $*" >&2
    exit 1
}

DISCOVER_RS="src/runtime/system_abilities/agents/discover.rs"

[[ -f "$DISCOVER_RS" ]] || fail "missing $DISCOVER_RS"

grep -q 'const ACCEPTED_SCOPE_LITERALS: &\[&str\] = &\["self", "device", "user", "public"\]' "$DISCOVER_RS" \
    || fail "discover scope enum must expose only self/device/user/public"

grep -q 'parse_scope(&json!({"scope": "easynet"})).is_err()' "$DISCOVER_RS" \
    || fail "discover tests must pin retired easynet scope alias as rejected"

grep -q 'fn user_scope_unjoined_returns_typed_error_envelope' "$DISCOVER_RS" \
    || fail "canonical user scope federation envelope test must exist"

bad="$(
    grep -nE 'scope.?=.?["\\]easynet|scope: "easynet"|scope=\\"easynet|back-compat alias for `user`|`easynet` is retained|easynet alias must canonicalise|easynet_scope_unjoined_returns_typed_error_envelope|ACCEPTED_SCOPE_LITERALS.*easynet|\"easynet\" \| \"user\"' \
        "$DISCOVER_RS" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "discover scope still advertises or accepts retired easynet alias:
$bad"
fi

echo "check-discover-scope-boundary: ok"
