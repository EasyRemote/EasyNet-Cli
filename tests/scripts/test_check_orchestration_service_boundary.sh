#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-orchestration-service-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-orchestration-service-boundary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/daemon/ability/builtins/automation"
    cp "$REPO_ROOT/src/daemon/ability/builtins/automation/orchestration.rs" "$sandbox/src/daemon/ability/builtins/automation/orchestration.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_ORCHESTRATION_SERVICE_BOUNDARY_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: orchestration service boundary should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/struct OrchestrationService/struct RetiredOrchestrationService/' "$SB/src/daemon/ability/builtins/automation/orchestration.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing OrchestrationService should exit 1 (got $rc)"

SB="$(make_sandbox)"
{
    echo 'static MAP: OnceLock<Mutex<HashMap<(String, String), String>>> = OnceLock::new();'
    echo 'fn agent_sessions() {}'
} >> "$SB/src/daemon/ability/builtins/automation/orchestration.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "global agent session map should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/struct AgentCycleRequest/struct RetiredAgentCycleRequest/' "$SB/src/daemon/ability/builtins/automation/orchestration.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing AgentCycleRequest should exit 1 (got $rc)"

SB="$(make_sandbox)"
echo '#[allow(clippy::too_many_arguments)]' >> "$SB/src/daemon/ability/builtins/automation/orchestration.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "long argument suppression should exit 1 (got $rc)"

echo "test_check_orchestration_service_boundary.sh: all cases passed"
