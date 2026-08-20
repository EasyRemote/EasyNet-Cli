#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-agent-registry-key-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
OUT="$SB/check-agent-registry-key-boundary.out"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" \
  "$SB/src/daemon/persistence" \
  "$SB/src/eal/interpreter" \
  "$SB/src/daemon/execution/mission" \
  "$SB/src/daemon/axon_bridge" \
  "$SB/src/daemon/ability" \
  "$SB/src/daemon/persistence"
cp "$SCRIPT" "$SB/tools/scripts/check-agent-registry-key-boundary.sh"

write_happy_fixture() {
  cat >"$SB/src/daemon/persistence/agent_registry.rs" <<'RS'
pub fn validate_agent_name(name: &str) {
    let agent_id = crate::core::agent::id::AgentId::parse(name).unwrap();
    let canonical = agent_id.to_string();
    if canonical != name {
        panic!("agent registry key {name:?} is not canonical; expected {canonical:?}");
    }
}
RS

  cat >"$SB/src/eal/interpreter/dispatch.rs" <<'RS'
pub fn validate_agent_target(agent_id: AgentId, registry: Registry) {
    let key = agent_id.to_string();
    registry.agents.get(&key);
}
RS

  cat >"$SB/src/daemon/execution/mission/agent_ability_specs.rs" <<'RS'
fn abilities_from_manifests(manifest: Manifest, surface_name: String) {
    manifest.qualified_name(&surface_name);
}

fn project_agent_surface_name(agent_identifier: &str) -> Option<String> {
    if agent_identifier.contains('/') {
        let agent_id = crate::core::agent::id::AgentId::parse(agent_identifier).ok()?;
        if agent_id.to_string() != agent_identifier {
            return None;
        }
        return Some(agent_id.name);
    }
    crate::core::agent::id::AgentId::new(crate::core::agent::id::DEFAULT_TENANT, agent_identifier)
        .ok()
        .map(|agent| agent.name)
}
RS

  cat >"$SB/src/daemon/axon_bridge/hot_agent_registrar.rs" <<'RS'
pub fn register_agent_replacing(name: &str) {
    let surface_name = hot_agent_runtime_surface_name(name).unwrap();
    let name = surface_name.as_str();
}

fn hot_agent_runtime_surface_name(agent_identifier: &str) -> Result<String, String> {
    if agent_identifier.contains('/') {
        let agent_id = crate::core::agent::id::AgentId::parse(agent_identifier)
            .map_err(|error| error.to_string())?;
        if agent_id.to_string() != agent_identifier {
            return Err(format!("registry key is not canonical; expected {:?}", agent_id.to_string()));
        }
        return Ok(agent_id.name);
    }
    crate::core::agent::id::AgentId::new(crate::core::agent::id::DEFAULT_TENANT, agent_identifier)
        .map(|agent| agent.name)
        .map_err(|error| error.to_string())
}
RS

  cat >"$SB/src/daemon/ability/dispatch.rs" <<'RS'
fn load(agent: &str, snapshot: Snapshot) {
    let registry_key = crate::core::agent::id::AgentId::parse(agent)
        .unwrap()
        .to_string();
    snapshot.has_registered_agent(&registry_key);
}
RS

  cat >"$SB/src/daemon/persistence/agent_aggregate.rs" <<'RS'
fn from_registry(owner_id: &str, registry: Registry) {
    let registry_key = AgentId::parse(owner_id).unwrap().to_string();
    registry.agents.get(&registry_key);
}

fn local_target_projection() -> AgentLocalTargetProjection {
    AgentLocalTargetProjection {
        registered_agent_ids: self.registered_agent_surface_names(),
    }
}
RS
}

assert_fails_with() {
  local expected="$1"
  set +e
  (
    cd "$SB"
    bash tools/scripts/check-agent-registry-key-boundary.sh
  ) >"$OUT" 2>&1
  local rc=$?
  set -e
  [[ "$rc" == "1" ]] || fail "expected gate failure exit 1, got $rc"
  grep -Fq "$expected" "$OUT" || fail "expected failure to mention: $expected"
}

write_happy_fixture
(
  cd "$SB"
  bash tools/scripts/check-agent-registry-key-boundary.sh
) >/dev/null || fail "happy path should pass"

write_happy_fixture
perl -0pi -e 's/let canonical = agent_id\.to_string\(\);\n    if canonical != name \{\n        panic!\("agent registry key \{name:\?\} is not canonical; expected \{canonical:\?\}"\);\n    \}\n//' "$SB/src/daemon/persistence/agent_registry.rs"
assert_fails_with "must compare against AgentId canonical string form"

write_happy_fixture
perl -0pi -e 's/registry\.agents\.get\(&key\);/registry.agents.get(&key).or_else(|| registry.agents.get(&agent_id.name));/' "$SB/src/eal/interpreter/dispatch.rs"
assert_fails_with "must not fallback to bare agent names"

write_happy_fixture
perl -0pi -e 's/manifest\.qualified_name\(&surface_name\);/manifest.qualified_name(registry_key);/' "$SB/src/daemon/execution/mission/agent_ability_specs.rs"
assert_fails_with "must qualify manifests with the agent surface name"

write_happy_fixture
perl -0pi -e 's/if agent_id\.to_string\(\) != agent_identifier \{\n            return None;\n        \}\n        return Some\(agent_id\.name\);/return Some(agent_id.name);/' "$SB/src/daemon/execution/mission/agent_ability_specs.rs"
assert_fails_with "must reject non-canonical registry-key shaped identifiers"

write_happy_fixture
perl -0pi -e 's/fn project_agent_surface_name\(agent_identifier: &str\) -> Option<String> \{.*?\n\}/fn project_agent_surface_name(registry_key: \\&str) -> String {\\n    crate::core::agent::id::AgentId::parse(registry_key)\\n        .map(|agent| agent.name)\\n        .unwrap_or_else(|_| registry_key.trim().to_string())\\n}/s' "$SB/src/daemon/execution/mission/agent_ability_specs.rs"
assert_fails_with "must name the identifier to surface-name boundary"

write_happy_fixture
perl -0pi -e 's/let surface_name = hot_agent_runtime_surface_name\(name\)\.unwrap\(\);\n    let name = surface_name\.as_str\(\);/let name = name.trim();/' "$SB/src/daemon/axon_bridge/hot_agent_registrar.rs"
assert_fails_with "must project persisted registry keys to runtime surface names"

write_happy_fixture
perl -0pi -e 's/snapshot\.has_registered_agent\(&registry_key\);/snapshot.has_registered_agent(agent);/' "$SB/src/daemon/ability/dispatch.rs"
assert_fails_with "must check the durable snapshot by canonical registry key"

write_happy_fixture
perl -0pi -e 's/let registry_key = AgentId::parse\(owner_id\)\.unwrap\(\)\.to_string\(\);/let registry_key = owner_id.to_string();/' "$SB/src/daemon/persistence/agent_aggregate.rs"
assert_fails_with "must canonicalize owner ids before durable registry lookup"

write_happy_fixture
perl -0pi -e 's/registered_agent_ids: self\.registered_agent_surface_names\(\),/registered_agent_ids: self.registry.agents.keys().cloned().collect(),/' "$SB/src/daemon/persistence/agent_aggregate.rs"
assert_fails_with "must expose surface agent ids, not durable registry keys"

echo "test_check_agent_registry_key_boundary.sh: all cases passed"
