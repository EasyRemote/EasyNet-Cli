#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

TARGET="src/daemon/ability/builtins/integrations/mcp/reflective_registry.rs"
[[ -f "$TARGET" ]] || fail "missing $TARGET"

if ! rg -n 'enum McpReflectionConcurrency' "$TARGET" >/dev/null; then
  fail "MCP reflective registry must use McpReflectionConcurrency state"
fi

if ! rg -n 'enum McpReflectionConcurrencyDefaultReason' "$TARGET" >/dev/null; then
  fail "MCP reflective registry must record defaulted concurrency reasons"
fi

for state in 'Configured\(usize\)' 'Defaulted\(McpReflectionConcurrencyDefaultReason\)' 'Missing' 'Empty' 'Invalid' 'NonPositive'; do
  if ! rg -n "$state" "$TARGET" >/dev/null; then
    fail "MCP reflection concurrency resolution missing state: $state"
  fi
done

for method in \
  'fn from_env\(\) -> Self' \
  'fn from_env_value\(raw: Option<&str>\) -> Self' \
  'fn limit\(&self\) -> usize'
do
  if ! rg -n "$method" "$TARGET" >/dev/null; then
    fail "McpReflectionConcurrency missing required method pattern: $method"
  fi
done

if ! rg -n 'McpReflectionConcurrency::from_env\(\)\.limit\(\)' "$TARGET" >/dev/null; then
  fail "McpReflectionSupervisor must consume concurrency through McpReflectionConcurrency"
fi

if rg -n 'fn mcp_reflection_concurrency\(|malformed values fall back|values fall back|reflection concurrency fallback|unwrap_or\(DEFAULT_MCP_REFLECTION_CONCURRENCY\)|and_then\(\|v\| v\.parse::<usize>\(\)\.ok\(\)\)' "$TARGET"; then
  fail "MCP reflection concurrency still uses fallback/default helper shape"
fi
