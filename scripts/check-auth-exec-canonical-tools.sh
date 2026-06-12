#!/usr/bin/env bash
#
# Guard `easynet auth exec` against retired device owner-prefixed aliases.

set -euo pipefail

ROOT="${CHECK_AUTH_EXEC_CANONICAL_TOOLS_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-auth-exec-canonical-tools: $*" >&2
    exit 1
}

AUTH_RS="src/facade/cli/auth.rs"
[[ -f "$AUTH_RS" ]] || fail "missing $AUTH_RS"

bad_language="$(
    grep -nEi 'accepted as legacy aliases|auto-prefixed|auto-prefix legacy|device owner prefix.*accepted' "$AUTH_RS" 2>/dev/null || true
)"
if [[ -n "$bad_language" ]]; then
    fail "auth exec still documents retired compatibility language:
$bad_language"
fi

bad_mapping="$(
    grep -nE '"shell\.run"[[:space:]]*=>[[:space:]]*"device\.shell\.run"|"process\.exec"[[:space:]]*=>[[:space:]]*"device\.process\.exec"' "$AUTH_RS" 2>/dev/null || true
)"
if [[ -n "$bad_mapping" ]]; then
    fail "auth exec still contains retired bare-tool alias mapping:
$bad_mapping"
fi

grep -q 'canonical_auth_exec_tool_name' "$AUTH_RS" \
    || fail "missing canonical auth exec tool-name validator"

grep -q 'public advertised ability name' "$AUTH_RS" \
    || fail "missing explicit rejection for retired device owner-prefixed aliases"

echo "check-auth-exec-canonical-tools: ok"
