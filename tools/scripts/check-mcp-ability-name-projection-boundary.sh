#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf 'check-mcp-ability-name-projection-boundary: %s\n' "$1" >&2
  exit 1
}

SOURCE="src/cli/commands/agent/mcp.rs"
TESTS="src/cli/commands/agent/tests.rs"

[[ -f "$SOURCE" ]] || fail "missing $SOURCE"
[[ -f "$TESTS" ]] || fail "missing $TESTS"

if ! rg -n 'enum McpAbilityNameProjection' "$SOURCE" >/dev/null; then
  fail "MCP ability naming must use an explicit projection state"
fi

for state in 'Flat\(String\)' 'DigestDisambiguated \{ digest_input: String \}'; do
  if ! rg -n "$state" "$SOURCE" >/dev/null; then
    fail "McpAbilityNameProjection missing state: $state"
  fi
done

for method in \
  'fn from_parts\(base: String, server: &str, tool: &str\) -> Self' \
  'fn requires_digest\(base: &str\) -> bool' \
  'fn into_name\(self\) -> String'
do
  if ! rg -n "$method" "$SOURCE" >/dev/null; then
    fail "McpAbilityNameProjection missing method: $method"
  fi
done

if ! python3 - "$SOURCE" <<'PY'
import re
import sys
from pathlib import Path

body = Path(sys.argv[1]).read_text()
helper = re.search(
    r"pub\(super\) fn generated_mcp_ability_name\([\s\S]*?\n\}",
    body,
)
if not helper:
    raise SystemExit("generated_mcp_ability_name_missing")
text = helper.group(0)
if "McpAbilityNameProjection::from_parts(base, server, tool).into_name()" not in text:
    raise SystemExit("generated_mcp_ability_name_bypasses_projection")
if "short_hex" in text:
    raise SystemExit("generated_mcp_ability_name_contains_digest_logic")
PY
then
  fail "generated MCP ability helper must delegate naming policy to projection state"
fi

if rg -n 'hash fallback|fallback prefix|falls_back_to_hash|fallback below|fallback guarantees' "$SOURCE" "$TESTS"; then
  fail "MCP ability naming still uses fallback vocabulary"
fi

if ! rg -n 'generated_mcp_ability_name_digest_disambiguates_empty_slug_projection' "$TESTS" >/dev/null; then
  fail "missing digest-disambiguation regression test"
fi

echo "check-mcp-ability-name-projection-boundary: ok"
