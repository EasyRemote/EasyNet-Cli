#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

CLI_STOP="src/cli/commands/stop.rs"
LIFECYCLE_STOP="src/daemon/boot/lifecycle/stop.rs"
LIFECYCLE_MOD="src/daemon/boot/lifecycle/mod.rs"

[[ -f "$CLI_STOP" ]] || fail "missing $CLI_STOP"
[[ -f "$LIFECYCLE_STOP" ]] || fail "missing $LIFECYCLE_STOP"
[[ -f "$LIFECYCLE_MOD" ]] || fail "missing $LIFECYCLE_MOD"

if ! rg -n 'struct RuntimeStopProcessController' "$LIFECYCLE_STOP" >/dev/null; then
  fail "daemon lifecycle stop must own RuntimeStopProcessController"
fi

for required in \
  'pub fn stop_pidfile_process' \
  'pub fn stop_discovered_daemon_process' \
  'pub fn sweep_stray_easynet_daemons' \
  'pub enum PidfileStopOutcome' \
  'pub enum LiveProcessStopOutcome' \
  'process_controller_reports_missing_pidfile_without_side_effects'
do
  if ! rg -n "$required" "$LIFECYCLE_STOP" >/dev/null; then
    fail "daemon lifecycle stop missing process boundary item: $required"
  fi
done

if ! rg -n 'RuntimeStopProcessController' "$LIFECYCLE_MOD" >/dev/null; then
  fail "lifecycle module must re-export RuntimeStopProcessController"
fi

if ! rg -n 'process_controller: RuntimeStopProcessController' "$CLI_STOP" >/dev/null; then
  fail "CLI stop must consume the lifecycle process controller instead of owning process logic"
fi

if rg -n 'fn stop_pidfile_process|fn stop_discovered_daemon_process|fn sweep_stray_easynet_daemons|std::process::Command::new\("pgrep"\)|net::kill_and_wait|net::is_pid_alive|net::is_easynet_process' "$CLI_STOP"; then
  fail "CLI stop must not own daemon process lifecycle probes or signaling"
fi

if rg -n 'legacy cleanup|no-state fallback|full runtime' "$LIFECYCLE_STOP" "$CLI_STOP"; then
  fail "runtime stop lifecycle must not preserve retired legacy/full-runtime stop vocabulary"
fi

echo "check-runtime-stop-lifecycle-boundary: ok"
