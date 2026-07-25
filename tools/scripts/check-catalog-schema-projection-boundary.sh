#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

CATALOG="src/daemon/ability/catalog/catalog_metadata.rs"
ASSEMBLY_TESTS="src/daemon/ability/catalog/assembly_tests.rs"
[[ -f "$CATALOG" ]] || fail "missing $CATALOG"
[[ -f "$ASSEMBLY_TESTS" ]] || fail "missing $ASSEMBLY_TESTS"

if ! rg -n 'enum CatalogSchemaProjection' "$CATALOG" >/dev/null; then
  fail "catalog metadata must own schema publication through CatalogSchemaProjection"
fi

for state in 'Declared\(serde_json::Value\)' 'UndeclaredObject'; do
  if ! rg -n "$state" "$CATALOG" >/dev/null; then
    fail "CatalogSchemaProjection is missing state: $state"
  fi
done

for method in \
  'fn for_input_name\(name: &str\) -> Self' \
  'fn declared_input_schema\(name: &str\) -> Option<serde_json::Value>' \
  'fn into_schema\(self\) -> serde_json::Value'
do
  if ! rg -n "$method" "$CATALOG" >/dev/null; then
    fail "CatalogSchemaProjection is missing required method pattern: $method"
  fi
done

if ! rg -n 'CatalogSchemaProjection::for_input_name\(name\)\.into_schema\(\)' "$CATALOG" >/dev/null; then
  fail "input_schema_for must delegate through CatalogSchemaProjection"
fi

if ! rg -n 'Self::UndeclaredObject => serde_json::json!\(\{ "type": "object" \}\)' "$CATALOG" >/dev/null; then
  fail "undeclared schema publication must be explicit in CatalogSchemaProjection"
fi

if rg -n 'Unknown names fall back|empty-object default|schema fallback|default fallback|unwrap_or_else\(\|\| serde_json::json!\(\{"type": "object"\}\)' "$CATALOG" "$ASSEMBLY_TESTS"; then
  fail "catalog schema publication still uses fallback/default vocabulary or local object fallback"
fi

if rg -n '_ => serde_json::json!\(\{ "type": "object" \}\)' "$CATALOG"; then
  fail "catalog metadata still has a bare match catch-all object schema"
fi
