#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_MISSION_RUNS_CLI_FACADE_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-mission-runs-cli-facade-boundary: %s\n' "$1" >&2
  exit 1
}

FACADE="src/cli/commands/mission_runs.rs"
ORCHESTRATION="src/daemon/execution/mission/orchestration.rs"

[[ -f "$FACADE" ]] || fail "missing $FACADE"
[[ -f "$ORCHESTRATION" ]] || fail "missing $ORCHESTRATION"

if rg -n 'orchestration::\*' "$FACADE"; then
  fail "CLI mission_runs facade must not wildcard re-export daemon orchestration"
fi

for retired in MissionRunner MissionRunOpts MissionRunResult MissionRunStore; do
  if rg -n "\\b${retired}\\b" "$FACADE"; then
    fail "CLI mission_runs facade must not expose daemon execution service type ${retired}"
  fi
done

if ! rg -n 'pub use crate::daemon::execution::mission::orchestration::\{' "$FACADE" >/dev/null; then
  fail "CLI mission_runs facade must use an explicit orchestration export list"
fi

for required in cancel_run find_run list_runs root_dir CancelOutcome MissionRunDir MissionRunMeta MissionRunStatus MissionRunSummary; do
  if ! rg -n "\\b${required}\\b" "$FACADE" >/dev/null; then
    fail "CLI mission_runs facade is missing required read/cancel projection ${required}"
  fi
done

echo "check-mission-runs-cli-facade-boundary: ok"
