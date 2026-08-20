#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_AGENT_REGISTRY_KEY_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-agent-registry-key-boundary: %s\n' "$1" >&2
  exit 1
}

REGISTRY="src/daemon/persistence/agent_registry.rs"
EAL_DISPATCH="src/eal/interpreter/dispatch.rs"
AGENT_SPECS="src/daemon/execution/mission/agent_ability_specs.rs"
HOT_REGISTRAR="src/daemon/axon_bridge/hot_agent_registrar.rs"
ABILITY_DISPATCH="src/daemon/ability/dispatch.rs"
AGENT_AGGREGATE="src/daemon/persistence/agent_aggregate.rs"

for file in "$REGISTRY" "$EAL_DISPATCH" "$AGENT_SPECS" "$HOT_REGISTRAR" "$ABILITY_DISPATCH" "$AGENT_AGGREGATE"; do
  [[ -f "$file" ]] || fail "missing $file"
done

if ! rg -n 'AgentId::parse\(name\)' "$REGISTRY" >/dev/null; then
  fail "agent registry validation must parse persisted keys through core::agent::id::AgentId"
fi

if ! rg -n 'agent_id\.to_string\(\)' "$REGISTRY" >/dev/null; then
  fail "agent registry validation must compare against AgentId canonical string form"
fi

if ! rg -n 'not canonical; expected' "$REGISTRY" >/dev/null; then
  fail "agent registry validation must reject non-canonical persisted keys"
fi

if ! rg -n 'let key = agent_id\.to_string\(\);' "$EAL_DISPATCH" >/dev/null; then
  fail "EAL agent target resolution must use the canonical registry key"
fi

if rg -n 'registry\.agents\.get\(&agent_id\.name\)|\.or_else\(' "$EAL_DISPATCH"; then
  fail "EAL agent target resolution must not fallback to bare agent names"
fi

if ! rg -n 'fn project_agent_surface_name\(agent_identifier: &str\) -> Option<String>' "$AGENT_SPECS" >/dev/null; then
  fail "agent ability projection must name the identifier to surface-name boundary"
fi

if ! rg -n 'agent_id\.to_string\(\) != agent_identifier' "$AGENT_SPECS" >/dev/null; then
  fail "agent ability projection must reject non-canonical registry-key shaped identifiers"
fi

if rg -n 'unwrap_or_else\(\|_\| registry_key\.trim\(\)\.to_string\(\)\)' "$AGENT_SPECS"; then
  fail "agent ability projection must not fallback from invalid registry keys to raw strings"
fi

if ! rg -n 'manifest\.qualified_name\(&surface_name\)' "$AGENT_SPECS" >/dev/null; then
  fail "agent ability projection must qualify manifests with the agent surface name"
fi

if ! rg -n 'hot_agent_runtime_surface_name\(name\)' "$HOT_REGISTRAR" >/dev/null; then
  fail "hot-agent registration must project persisted registry keys to runtime surface names"
fi

if ! rg -n 'let name = surface_name\.as_str\(\);' "$HOT_REGISTRAR" >/dev/null; then
  fail "hot-agent registration must use the projected surface name for runtime rows"
fi

if rg -n 'unwrap_or_else\(\|_\| name\.trim\(\)\.to_string\(\)\)' "$HOT_REGISTRAR"; then
  fail "hot-agent registration must not fallback from invalid registry keys to raw strings"
fi

if ! rg -n 'let registry_key = crate::core::agent::id::AgentId::parse\(agent\)' "$ABILITY_DISPATCH" >/dev/null; then
  fail "hot-agent authority enrollment must verify the durable registry key projection"
fi

if ! rg -n 'snapshot\.has_registered_agent\(&registry_key\)' "$ABILITY_DISPATCH" >/dev/null; then
  fail "hot-agent authority enrollment must check the durable snapshot by canonical registry key"
fi

if ! rg -n 'let registry_key = AgentId::parse\(owner_id\)' "$AGENT_AGGREGATE" >/dev/null; then
  fail "Agent aggregate workspace lookup must canonicalize owner ids before durable registry lookup"
fi

if ! rg -n 'registered_agent_ids: self\.registered_agent_surface_names\(\)' "$AGENT_AGGREGATE" >/dev/null; then
  fail "Agent local target projection must expose surface agent ids, not durable registry keys"
fi

if rg -n 'registered_agent_ids: self\.registry\.agents\.keys\(\)\.cloned\(\)\.collect\(\)' "$AGENT_AGGREGATE"; then
  fail "Agent local target projection must not publish durable registry keys as runtime agent ids"
fi

echo "check-agent-registry-key-boundary: ok"
