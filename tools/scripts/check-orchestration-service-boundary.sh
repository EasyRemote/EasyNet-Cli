#!/usr/bin/env bash
#
# Guard mission.discuss_round orchestration state against global stores.

set -euo pipefail

ROOT="${CHECK_ORCHESTRATION_SERVICE_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-orchestration-service-boundary: $*" >&2
    exit 1
}

ORCH_RS="src/daemon/ability/builtins/automation/orchestration.rs"

[[ -f "$ORCH_RS" ]] || fail "missing $ORCH_RS"

grep -q 'struct OrchestrationService' "$ORCH_RS" \
    || fail "mission.discuss_round state must be owned by OrchestrationService"

grep -q 'agent_sessions: Mutex<HashMap<(String, String), String>>' "$ORCH_RS" \
    || fail "per-agent chat session continuity must live inside OrchestrationService"

grep -q 'let service = Arc::new(OrchestrationService::new' "$ORCH_RS" \
    || fail "registry must register closures over one explicit orchestration service"

grep -q 'struct AgentCycleRequest' "$ORCH_RS" \
    || fail "agent cycle inputs must stay grouped in AgentCycleRequest"

bad="$(
    grep -nE 'fn agent_sessions\(|static MAP|OnceLock<Mutex|lazy_static|discuss_round_handler|allow\(clippy::too_many_arguments\)' \
        "$ORCH_RS" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "orchestration surface still carries retired global-state or long-argument plumbing:
$bad"
fi

echo "check-orchestration-service-boundary: ok"
