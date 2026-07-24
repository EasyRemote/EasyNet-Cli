#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_NAMESPACE_RESOLVE_QTYPE_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-namespace-resolve-qtype-boundary: %s\n' "$1" >&2
  exit 1
}

RESOLVER="src/daemon/invocation/routing/route_resolver.rs"
[[ -f "$RESOLVER" ]] || fail "missing $RESOLVER"

if ! rg -n "fn json_resolve_type\\(value: &Value\\) -> Result<ResolveType, &" "$RESOLVER" >/dev/null; then
  fail "local namespace.resolve qtype parser must return a typed Result"
fi

if ! rg -n 'ok_or\("resolve query missing canonical qtype"\)' "$RESOLVER" >/dev/null; then
  fail "local namespace.resolve must reject missing qtype before shape guessing"
fi

if ! rg -n 'ResolveType::from_str_name\(text\)' "$RESOLVER" >/dev/null; then
  fail "local namespace.resolve must parse canonical ResolveType enum strings directly"
fi

if rg -n 'ResolveType::try_from\(num as i32\)|format!\("RESOLVE_TYPE_\{\}"|to_ascii_uppercase\(\)|unwrap_or_else\(\|\| \{' "$RESOLVER"; then
  fail "local namespace.resolve must not accept numeric/short qtype aliases or guess qtype from query shape"
fi

for required in \
  'resolve_query_json_rejects_missing_qtype_instead_of_shape_guessing' \
  'resolve_query_json_rejects_short_qtype_aliases'; do
  if ! rg -n "$required" "$RESOLVER" >/dev/null; then
    fail "route resolver is missing regression test ${required}"
  fi
done

echo "check-namespace-resolve-qtype-boundary: ok"
