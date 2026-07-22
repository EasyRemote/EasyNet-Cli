#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

START="src/cli/commands/start.rs"
WATCHER="src/cli/commands/start_boot_watcher.rs"

[[ -f "$START" ]] || fail "missing $START"
[[ -f "$WATCHER" ]] || fail "missing $WATCHER"

if ! rg -n 'ready_capability_flags: Vec<String>' "$WATCHER" >/dev/null; then
  fail "BootProgressOutcome must carry daemon Ready capability flags"
fi

if ! rg -n 'has_ready_capability_flag' "$WATCHER" >/dev/null; then
  fail "BootProgressOutcome must expose typed ready capability lookup"
fi

if ! rg -n 'outcome\.ready_capability_flags = disc\.capability_flags\.clone\(\)' "$WATCHER" >/dev/null; then
  fail "start boot watcher must capture capability flags from control discovery on Ready"
fi

if ! rg -n 'fn validate_device_ready_capabilities' "$START" >/dev/null; then
  fail "device start must validate daemon Ready signer capability before success"
fi

if ! rg -n 'PAIRED_USER_RUNTIME_SIGNER' "$START" >/dev/null; then
  fail "device start must require paired_user_runtime_signer readiness"
fi

python3 - "$START" <<'PY'
import pathlib
import sys

text = pathlib.Path(sys.argv[1]).read_text()
validate = text.find("validate_device_ready_capabilities(&boot)")
save = text.find("save_runtime_projection_after_ready(&mut daemon_handle")
welcome = text.find('console::style("Welcome,")')
stop = text.find("daemon_handle.stop()")

if validate == -1:
    raise SystemExit("missing validate_device_ready_capabilities call")
if save == -1:
    raise SystemExit("missing save_runtime_projection_after_ready")
if welcome == -1:
    raise SystemExit("missing Welcome surface")
if not (validate < save < welcome):
    raise SystemExit("ready signer validation must precede projection persistence and Welcome")
if stop == -1 or not (validate < stop < save):
    raise SystemExit("fresh daemon must be stopped when signer readiness validation fails")
PY

for required_test in \
  start_ready_capability_accepts_paired_user_signer \
  start_ready_capability_rejects_missing_paired_user_signer
do
  if ! rg -n "$required_test" "$START" >/dev/null; then
    fail "start ready signer proof missing test: $required_test"
  fi
done

echo "check-start-ready-signer-proof-boundary: ok"
