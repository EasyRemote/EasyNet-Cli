#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-mission-runs-cli-facade-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" \
  "$SB/src/cli/commands" \
  "$SB/src/daemon/execution/mission"
cp "$SCRIPT" "$SB/tools/scripts/check-mission-runs-cli-facade-boundary.sh"

cat >"$SB/src/daemon/execution/mission/orchestration.rs" <<'RS'
pub struct MissionRunner;
pub struct MissionRunOpts;
pub struct MissionRunResult;
pub struct MissionRunStore;
pub struct MissionRunDir;
pub struct MissionRunMeta;
pub struct MissionRunSummary;
pub enum MissionRunStatus { Running }
pub enum CancelOutcome { Cancelled }
pub fn root_dir() {}
pub fn list_runs() {}
pub fn find_run(id: &str) {}
pub fn cancel_run(id: &str) {}
RS

cat >"$SB/src/cli/commands/mission_runs.rs" <<'RS'
pub use crate::daemon::execution::mission::orchestration::{
    cancel_run, find_run, list_runs, root_dir, CancelOutcome, MissionRunDir, MissionRunMeta,
    MissionRunStatus, MissionRunSummary,
};
RS

(
  cd "$SB"
  bash tools/scripts/check-mission-runs-cli-facade-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >"$SB/src/cli/commands/mission_runs.rs" <<'RS'
pub use crate::daemon::execution::mission::orchestration::*;
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-mission-runs-cli-facade-boundary.sh
) >/tmp/check-mission-runs-cli-facade-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "wildcard re-export should exit 1 (got $rc)"
grep -Fq "must not wildcard re-export daemon orchestration" \
  /tmp/check-mission-runs-cli-facade-boundary.out \
  || fail "wildcard failure should name daemon orchestration"

cat >"$SB/src/cli/commands/mission_runs.rs" <<'RS'
pub use crate::daemon::execution::mission::orchestration::{
    cancel_run, find_run, list_runs, root_dir, CancelOutcome, MissionRunDir, MissionRunMeta,
    MissionRunResult, MissionRunStatus, MissionRunSummary,
};
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-mission-runs-cli-facade-boundary.sh
) >/tmp/check-mission-runs-cli-facade-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "execution service export should exit 1 (got $rc)"
grep -Fq "must not expose daemon execution service type MissionRunResult" \
  /tmp/check-mission-runs-cli-facade-boundary.out \
  || fail "execution type failure should name MissionRunResult"

echo "test_check_mission_runs_cli_facade_boundary.sh: all cases passed"
