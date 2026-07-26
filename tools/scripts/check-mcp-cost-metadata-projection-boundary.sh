#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

MCP_PROFILE="src/daemon/ability/catalog/profiles/mcp.rs"
[[ -f "$MCP_PROFILE" ]] || fail "missing $MCP_PROFILE"

if ! rg -n 'enum CostMetadataProjection' "$MCP_PROFILE" >/dev/null; then
  fail "MCP profile must use CostMetadataProjection for tool cost display"
fi

for state in 'Declared \{' 'Undeclared'; do
  if ! rg -n "$state" "$MCP_PROFILE" >/dev/null; then
    fail "CostMetadataProjection is missing state: $state"
  fi
done

if rg -n 'UndeclaredKnownLlm|known.*LLM|agent:.*llm_metered|source\.starts_with\("agent:"\).*cost|metadata\.contains_key\("exec_kind"\).*cost' "$MCP_PROFILE"; then
  fail "MCP cost projection must not infer billing class from source/exec heuristics"
fi

if ! rg -n 'CostMetadataProjection::from_descriptor\(descriptor\)' "$MCP_PROFILE" >/dev/null; then
  fail "MCP tool projection must derive cost through CostMetadataProjection"
fi

if ! rg -n 'extension\.insert\("cost_kind"\.to_string\(\), cost\.kind\(\)\.into\(\)\)' "$MCP_PROFILE" >/dev/null; then
  fail "MCP x-easynet.cost_kind must come from CostMetadataProjection"
fi

if ! rg -n 'extension\.insert\("cost_label"\.to_string\(\), cost\.label\(\)\.into\(\)\)' "$MCP_PROFILE" >/dev/null; then
  fail "MCP x-easynet.cost_label must come from CostMetadataProjection"
fi

if ! rg -n 'Self::Undeclared => "unknown"' "$MCP_PROFILE" >/dev/null; then
  fail "Undeclared MCP cost must project to unknown"
fi

if ! rg -n 'Self::Undeclared => "cost not declared"' "$MCP_PROFILE" >/dev/null; then
  fail "Undeclared MCP cost label must stay explicit"
fi

if rg -n 'inferred_cost|fallback cost|cost fallback|default to[[:space:]]+`?free|cost defaults to|Cost defaults to' "$MCP_PROFILE"; then
  fail "MCP profile cost projection still uses inferred/fallback/default-cost vocabulary"
fi

if rg -n 'unwrap_or_else' "$MCP_PROFILE" | rg -n 'cost'; then
  fail "MCP profile cost projection must not use ad-hoc unwrap_or_else cost defaults"
fi
