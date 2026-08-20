#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-runtime-stop-lifecycle-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/cli/commands" "$SB/src/daemon/boot/lifecycle"
cp "$SCRIPT" "$SB/tools/scripts/check-runtime-stop-lifecycle-boundary.sh"

cat >"$SB/src/daemon/boot/lifecycle/stop.rs" <<'RS'
pub struct RuntimeStopProcessController;

impl RuntimeStopProcessController {
    pub fn stop_pidfile_process(&self) {}
    pub fn stop_discovered_daemon_process(&self) {}
    pub fn sweep_stray_easynet_daemons(&self) {}
}

pub enum PidfileStopOutcome {
    NoPidfile,
}

pub enum LiveProcessStopOutcome {
    StalePid,
}

#[test]
fn process_controller_reports_missing_pidfile_without_side_effects() {}
RS

cat >"$SB/src/daemon/boot/lifecycle/mod.rs" <<'RS'
pub use stop::RuntimeStopProcessController;
RS

cat >"$SB/src/cli/commands/stop.rs" <<'RS'
use crate::daemon::lifecycle::RuntimeStopProcessController;

struct StopPlan {
    process_controller: RuntimeStopProcessController,
}
RS

(
  cd "$SB"
  bash tools/scripts/check-runtime-stop-lifecycle-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >>"$SB/src/cli/commands/stop.rs" <<'RS'
fn stop_pidfile_process() {}
fn sweep_stray_easynet_daemons() {
    let _ = std::process::Command::new("pgrep");
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-runtime-stop-lifecycle-boundary.sh
) >/tmp/check-runtime-stop-lifecycle-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "CLI process lifecycle ownership should exit 1 (got $rc)"

echo "test_check_runtime_stop_lifecycle_boundary.sh: all cases passed"
