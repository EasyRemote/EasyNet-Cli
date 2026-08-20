#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-core-agent-module-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/core/agent" "$SB/src/daemon"
cp "$SCRIPT" "$SB/tools/scripts/check-core-agent-module-boundary.sh"

cat >"$SB/src/core/mod.rs" <<'RS'
pub mod agent;
pub mod domain;
pub mod identity;
pub mod ura;
RS

cat >"$SB/src/core/agent/id.rs" <<'RS'
/// Product-neutral Agent identity.
///
/// URI-shaped/URA-shaped inputs belong to `crate::core::ura`, the L3 canonical runtime identity layer.
pub struct AgentId(String);
RS

cat >"$SB/src/daemon/current.rs" <<'RS'
use crate::core::agent::spec::AgentSpec;

fn load(spec: AgentSpec) -> AgentSpec {
    spec
}
RS

mkdir -p "$SB/tests"
cat >"$SB/tests/current.rs" <<'RS'
use easynet_cli::core::agent::id::AgentId;
RS

(
  cd "$SB"
  bash tools/scripts/check-core-agent-module-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >>"$SB/src/core/mod.rs" <<'RS'
pub use agent::id as agent_id;
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-core-agent-module-boundary.sh
) >/tmp/check-core-agent-module-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "module alias should exit 1 (got $rc)"
grep -Fq "compatibility aliases" /tmp/check-core-agent-module-boundary.out \
  || fail "module alias failure should name compatibility aliases"

perl -0pi -e 's/pub use agent::id as agent_id;\n//' "$SB/src/core/mod.rs"
cat >>"$SB/src/daemon/current.rs" <<'RS'
use crate::core::agent_spec::AgentSpec as RetiredAgentSpec;
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-core-agent-module-boundary.sh
) >/tmp/check-core-agent-module-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "retired caller alias should exit 1 (got $rc)"
grep -Fq "production callers must not use retired core agent module aliases" \
  /tmp/check-core-agent-module-boundary.out \
  || fail "caller alias failure should name retired aliases"

echo "test_check_core_agent_module_boundary.sh: all cases passed"
