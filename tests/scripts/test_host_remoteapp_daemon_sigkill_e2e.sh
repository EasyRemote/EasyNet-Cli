#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/host-remoteapp-daemon-sigkill-e2e.sh"

fail() {
  echo "test_host_remoteapp_daemon_sigkill_e2e: $*" >&2
  exit 1
}

bash -n "$SCRIPT"
OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$SCRIPT" --self-test --out-dir "$OUT_DIR/self-test" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/self-test/report.json" || \
  fail "self-test report did not pass"
grep -q '"proof_mode": "real_active_session_daemon_sigkill"' "$OUT_DIR/self-test/evidence.json" || \
  fail "self-test did not project daemon SIGKILL proof"

MOCK_STATE="$OUT_DIR/mock-state"
mkdir -p "$MOCK_STATE"
MOCK_EASYNET="$OUT_DIR/mock-easynet"
MOCK_PS="$OUT_DIR/mock-ps"
MOCK_KILL="$OUT_DIR/mock-kill"
cat >"$MOCK_EASYNET" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$1 $2" == "runtime start" ]]; then
  : >"$MOCK_RUNTIME_STARTED"
  exit 0
fi
if [[ "$1 $2 $3" == "runtime status --json" ]]; then
  if [[ "${MOCK_STATUS_PID_MODE:-valid}" == "missing" ]]; then
    echo '{"daemon":{},"connection":{"state":"FRONTEND_CONNECTED","state_code":"J800"},"product_presence":{"session_admitted":true}}'
    exit 0
  fi
  if [[ -f "$MOCK_RUNTIME_STARTED" ]]; then
    echo '{"daemon":{"pid":999992},"connection":{"state":"FRONTEND_CONNECTED","state_code":"J800"},"product_presence":{"session_admitted":true},"runtime":{"process_kind":"easynet_daemon"}}'
  else
    echo '{"daemon":{"pid":999991},"connection":{"state":"FRONTEND_CONNECTED","state_code":"J800"},"product_presence":{"session_admitted":true},"runtime":{"process_kind":"easynet_daemon"}}'
  fi
  exit 0
fi
exit 64
SH
cat >"$MOCK_PS" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
echo "${MOCK_PROCESS_COMMAND:?}"
SH
cat >"$MOCK_KILL" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >"$MOCK_KILL_RECORD"
SH
chmod +x "$MOCK_EASYNET" "$MOCK_PS" "$MOCK_KILL"

export MOCK_RUNTIME_STARTED="$OUT_DIR/runtime-started"
export MOCK_KILL_RECORD="$OUT_DIR/kill-record"
export MOCK_PROCESS_COMMAND="/wrong/easynet-daemon"
if EASYNET_REMOTEAPP_CRASH_EASYNET_BIN="$MOCK_EASYNET" \
   EASYNET_REMOTEAPP_CRASH_PS_BIN="$MOCK_PS" \
   EASYNET_REMOTEAPP_CRASH_KILL_BIN="$MOCK_KILL" \
   EASYNET_REMOTEAPP_CRASH_EXPECTED_DAEMON="/expected/easynet-daemon" \
   "$SCRIPT" --fixture-kill "$MOCK_STATE/wrong" >"$OUT_DIR/wrong-path.out" 2>&1; then
  fail "wrong daemon process path was accepted"
fi
[[ ! -f "$MOCK_KILL_RECORD" ]] || fail "wrong-path fixture sent a signal"

if MOCK_STATUS_PID_MODE=missing \
   EASYNET_REMOTEAPP_CRASH_EASYNET_BIN="$MOCK_EASYNET" \
   EASYNET_REMOTEAPP_CRASH_PS_BIN="$MOCK_PS" \
   EASYNET_REMOTEAPP_CRASH_KILL_BIN="$MOCK_KILL" \
   EASYNET_REMOTEAPP_CRASH_EXPECTED_DAEMON="/expected/easynet-daemon" \
   "$SCRIPT" --fixture-kill "$MOCK_STATE/missing-pid" >"$OUT_DIR/missing-pid.out" 2>&1; then
  fail "missing daemon PID was accepted"
fi
[[ ! -f "$MOCK_KILL_RECORD" ]] || fail "missing-PID fixture sent a signal"

export MOCK_PROCESS_COMMAND="/expected/easynet-daemon"
EASYNET_REMOTEAPP_CRASH_EASYNET_BIN="$MOCK_EASYNET" \
EASYNET_REMOTEAPP_CRASH_PS_BIN="$MOCK_PS" \
EASYNET_REMOTEAPP_CRASH_KILL_BIN="$MOCK_KILL" \
EASYNET_REMOTEAPP_CRASH_EXPECTED_DAEMON="/expected/easynet-daemon" \
EASYNET_REMOTEAPP_CRASH_REQUIRE_SOCKET_PROOF=0 \
"$SCRIPT" --fixture-kill "$MOCK_STATE/good"
grep -q -- '-KILL 999991' "$MOCK_KILL_RECORD" || fail "exact old daemon PID was not signalled"
grep -q '"status": "killed"' "$MOCK_STATE/good/crash.json" || fail "kill fact was not recorded"

EASYNET_REMOTEAPP_CRASH_EASYNET_BIN="$MOCK_EASYNET" \
EASYNET_REMOTEAPP_CRASH_PS_BIN="$MOCK_PS" \
EASYNET_REMOTEAPP_CRASH_KILL_BIN="$MOCK_KILL" \
EASYNET_REMOTEAPP_CRASH_EXPECTED_DAEMON="/expected/easynet-daemon" \
EASYNET_REMOTEAPP_CRASH_REQUIRE_SOCKET_PROOF=0 \
"$SCRIPT" --fixture-restart "$MOCK_STATE/good"
grep -q '"new_pid": 999992' "$MOCK_STATE/good/restart.json" || fail "new daemon PID was not recorded"
grep -q '"state_code": "J800"' "$MOCK_STATE/good/restart.json" || fail "J800 restart was not recorded"
grep -q '"new_process_command": "/expected/easynet-daemon"' "$MOCK_STATE/good/restart.json" || \
  fail "restarted daemon path was not recorded"

echo "test_host_remoteapp_daemon_sigkill_e2e: ok"
