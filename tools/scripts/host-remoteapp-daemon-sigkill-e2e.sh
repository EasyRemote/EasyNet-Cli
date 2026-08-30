#!/usr/bin/env bash
# Real Browser RemoteApp recovery across an exact-PID daemon SIGKILL.
#
# This is one scenario of the larger crash/restart matrix. It owns only the
# bounded host process interruption/restart fixture and composes the canonical
# Browser lifecycle runner/verifier for product behavior.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
FRONTEND_ROOT="${EASYNET_REMOTEAPP_FRONTEND_ROOT:-$REPO_ROOT/../EasyNet/Frontend}"
RUNNER="$FRONTEND_ROOT/scripts/remoteapp-browser-lifecycle.mjs"
BROWSER_VERIFIER="$SELF_DIR/frontend-remoteapp-browser-lifecycle-e2e.sh"
EASYNET_BIN="${EASYNET_REMOTEAPP_CRASH_EASYNET_BIN:-$REPO_ROOT/target/debug/easynet}"
EXPECTED_DAEMON="${EASYNET_REMOTEAPP_CRASH_EXPECTED_DAEMON:-$REPO_ROOT/target/debug/easynet-daemon}"
PS_BIN="${EASYNET_REMOTEAPP_CRASH_PS_BIN:-ps}"
KILL_BIN="${EASYNET_REMOTEAPP_CRASH_KILL_BIN:-kill}"

MODE=skip
TARGET_KIND=window
OUT_DIR="${EASYNET_REMOTEAPP_DAEMON_SIGKILL_OUT_DIR:-$REPO_ROOT/target/e2e/host-remoteapp-daemon-sigkill/$(date -u +%Y%m%d-%H%M%S)-$$}"
INTERNAL_MODE=""
FIXTURE_STATE_DIR=""

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-daemon-sigkill-e2e.sh --run [--target-kind window|application]
  host-remoteapp-daemon-sigkill-e2e.sh --self-test

Options:
  --run                 Execute a real Browser session and SIGKILL the exact
                        verified local debug daemon while the session is active.
  --self-test           Validate the aggregate crash evidence contract only.
  --target-kind KIND    window or application. Default: window.
  --out-dir DIR         Output directory.
  -h, --help            Show this help.

Required live environment:
  EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL
  EASYNET_REMOTEAPP_BROWSER_DEVICE_ID
  EASYNET_REMOTEAPP_BROWSER_EMAIL
  EASYNET_REMOTEAPP_BROWSER_PASSWORD

Optional test/installation overrides:
  EASYNET_REMOTEAPP_FRONTEND_ROOT
  EASYNET_REMOTEAPP_CRASH_EASYNET_BIN
  EASYNET_REMOTEAPP_CRASH_EXPECTED_DAEMON
  EASYNET_REMOTEAPP_CRASH_PS_BIN
  EASYNET_REMOTEAPP_CRASH_KILL_BIN
  EASYNET_REMOTEAPP_CRASH_REQUIRE_SOCKET_PROOF
                        Internal fixture-test override. Live aggregate
                        validation always requires real socket proof.

Non-claim:
  This proves active-session daemon-process recovery and Unix stale-socket
  replacement only. It does not prove plugin-worker restart,
  crash-during-close receipt replay, Windows named-pipe restart, successful OS
  input injection, or cross-device recovery.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
    --target-kind)
      case "${2:?missing value for --target-kind}" in
        window|application) TARGET_KIND="$2" ;;
        *) echo "invalid target kind: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --fixture-kill) INTERNAL_MODE=kill; FIXTURE_STATE_DIR="${2:?missing fixture state dir}"; shift 2 ;;
    --fixture-restart) INTERNAL_MODE=restart; FIXTURE_STATE_DIR="${2:?missing fixture state dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

unix_ms_now() {
  python3 - <<'PY'
import time
print(int(time.time() * 1000))
PY
}

require_safe_state_dir() {
  local path="$1"
  [[ -n "$path" && "$path" == /* ]] || {
    echo "fixture state dir must be an absolute path" >&2
    exit 64
  }
  case "$path" in
    /|"${HOME:-/nonexistent}"|"$REPO_ROOT")
      echo "refusing broad fixture state dir: $path" >&2
      exit 64
      ;;
  esac
  mkdir -p "$path"
}

runtime_pid_from_status() {
  python3 - "$1" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    status = json.load(handle)
pid = status.get("daemon", {}).get("pid")
if not isinstance(pid, int) or isinstance(pid, bool) or pid <= 1:
    raise SystemExit("runtime status did not expose one positive daemon pid")
print(pid)
PY
}

socket_snapshot_from_status() {
  local status_path="$1"
  local output_path="$2"
  python3 - "$status_path" "$output_path" <<'PY'
import json
import os
import pathlib
import socket
import stat
import sys
import time

status_path, output_path = map(pathlib.Path, sys.argv[1:3])
status = json.loads(status_path.read_text(encoding="utf-8"))
daemon = status.get("daemon") if isinstance(status.get("daemon"), dict) else {}

def snapshot(path):
    value = {
        "path": path,
        "absolute": isinstance(path, str) and os.path.isabs(path),
        "exists": False,
        "is_socket": False,
        "inode": None,
        "connectable": False,
        "connect_error": None,
    }
    if not isinstance(path, str) or not path:
        value["connect_error"] = "endpoint path missing from public runtime status"
        return value
    try:
        metadata = os.lstat(path)
        value["exists"] = True
        value["is_socket"] = stat.S_ISSOCK(metadata.st_mode)
        value["inode"] = metadata.st_ino
    except OSError as error:
        value["connect_error"] = f"lstat: {error}"
        return value
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(0.25)
    try:
        client.connect(path)
        value["connectable"] = True
        value["connect_error"] = None
    except OSError as error:
        value["connect_error"] = f"connect: {error}"
    finally:
        client.close()
    return value

value = {
    "observed_at_ms": int(time.time() * 1000),
    "control": snapshot(daemon.get("control_socket")),
    "invocation": snapshot(daemon.get("invocation_endpoint")),
}
output_path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

fixture_kill() {
  require_safe_state_dir "$FIXTURE_STATE_DIR"
  local status_path="$FIXTURE_STATE_DIR/runtime-before.json"
  local crash_path="$FIXTURE_STATE_DIR/crash.json"
  "$EASYNET_BIN" runtime status --json >"$status_path"
  local sockets_before_path="$FIXTURE_STATE_DIR/sockets-before.json"
  socket_snapshot_from_status "$status_path" "$sockets_before_path"
  local daemon_pid
  daemon_pid="$(runtime_pid_from_status "$status_path")"
  local process_command
  process_command="$($PS_BIN -p "$daemon_pid" -o command= | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
  case "$process_command" in
    "$EXPECTED_DAEMON"|"$EXPECTED_DAEMON "*) ;;
    *)
      echo "refusing SIGKILL: pid $daemon_pid is not $EXPECTED_DAEMON" >&2
      exit 72
      ;;
  esac
  local signal_at_ms
  signal_at_ms="$(unix_ms_now)"
  python3 - "$crash_path" "$sockets_before_path" "$daemon_pid" "$process_command" "$EXPECTED_DAEMON" "$signal_at_ms" <<'PY'
import json
import pathlib
import sys

path, sockets_before_path, pid, command, expected, signal_at_ms = sys.argv[1:7]
value = {
    "kind": "daemon_process_sigkill",
    "status": "signal_pending",
    "old_pid": int(pid),
    "process_command": command,
    "expected_daemon_path": expected,
    "signal": "SIGKILL",
    "signal_at_ms": int(signal_at_ms),
    "socket_state_before": json.load(open(sockets_before_path, encoding="utf-8")),
}
pathlib.Path(path).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  "$KILL_BIN" -KILL "$daemon_pid"
  local attempt
  for attempt in $(seq 1 50); do
    if ! kill -0 "$daemon_pid" 2>/dev/null; then
      break
    fi
    sleep 0.1
  done
  if kill -0 "$daemon_pid" 2>/dev/null; then
    echo "daemon pid $daemon_pid remained alive after SIGKILL" >&2
    exit 73
  fi
  local observed_dead_at_ms
  observed_dead_at_ms="$(unix_ms_now)"
  local sockets_after_kill_path="$FIXTURE_STATE_DIR/sockets-after-kill.json"
  socket_snapshot_from_status "$status_path" "$sockets_after_kill_path"
  python3 - "$crash_path" "$sockets_after_kill_path" "$observed_dead_at_ms" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
value = json.loads(path.read_text(encoding="utf-8"))
value["status"] = "killed"
value["socket_state_after_kill"] = json.load(open(sys.argv[2], encoding="utf-8"))
value["observed_dead_at_ms"] = int(sys.argv[3])
path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

fixture_restart() {
  require_safe_state_dir "$FIXTURE_STATE_DIR"
  local crash_path="$FIXTURE_STATE_DIR/crash.json"
  local restart_path="$FIXTURE_STATE_DIR/restart.json"
  [[ -f "$crash_path" ]] || { echo "missing crash fixture state" >&2; exit 74; }
  local old_pid
  old_pid="$(python3 - "$crash_path" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
pid = value.get("old_pid")
if not isinstance(pid, int) or pid <= 1:
    raise SystemExit("crash fixture missing old_pid")
print(pid)
PY
)"
  "$EASYNET_BIN" runtime start >"$FIXTURE_STATE_DIR/restart.stdout.txt" 2>"$FIXTURE_STATE_DIR/restart.stderr.txt"
  local status_path="$FIXTURE_STATE_DIR/runtime-after.json"
  local attempt
  local online=0
  for attempt in $(seq 1 120); do
    if "$EASYNET_BIN" runtime status --json >"$status_path" 2>/dev/null && \
       python3 - "$status_path" "$old_pid" <<'PY'
import json
import sys

status = json.load(open(sys.argv[1], encoding="utf-8"))
old_pid = int(sys.argv[2])
new_pid = status.get("daemon", {}).get("pid")
ready = (
    isinstance(new_pid, int)
    and new_pid > 1
    and new_pid != old_pid
    and status.get("connection", {}).get("state_code") == "J800"
    and status.get("product_presence", {}).get("session_admitted") is True
)
raise SystemExit(0 if ready else 1)
PY
    then
      online=1
      break
    fi
    sleep 0.25
  done
  [[ "$online" == 1 ]] || { echo "restarted daemon did not reach J800" >&2; exit 75; }
  local new_pid
  new_pid="$(runtime_pid_from_status "$status_path")"
  local new_process_command
  new_process_command="$($PS_BIN -p "$new_pid" -o command= | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
  case "$new_process_command" in
    "$EXPECTED_DAEMON"|"$EXPECTED_DAEMON "*) ;;
    *)
      echo "restarted pid $new_pid is not $EXPECTED_DAEMON" >&2
      exit 76
      ;;
  esac
  local ready_at_ms
  ready_at_ms="$(unix_ms_now)"
  local sockets_after_restart_path="$FIXTURE_STATE_DIR/sockets-after-restart.json"
  socket_snapshot_from_status "$status_path" "$sockets_after_restart_path"
  local require_socket_proof="${EASYNET_REMOTEAPP_CRASH_REQUIRE_SOCKET_PROOF:-1}"
  python3 - "$restart_path" "$crash_path" "$sockets_after_restart_path" "$status_path" "$old_pid" "$new_process_command" "$EXPECTED_DAEMON" "$ready_at_ms" "$require_socket_proof" <<'PY'
import json
import pathlib
import sys

(
    path,
    crash_path,
    sockets_after_restart_path,
    status_path,
    old_pid,
    process_command,
    expected,
    ready_at_ms,
    require_socket_proof,
) = sys.argv[1:10]
status = json.load(open(status_path, encoding="utf-8"))
crash = json.load(open(crash_path, encoding="utf-8"))
after_restart = json.load(open(sockets_after_restart_path, encoding="utf-8"))
new_pid = status["daemon"]["pid"]
proof_required = require_socket_proof != "0"
errors = []
socket_rows = {}
for name in ("control", "invocation"):
    before = crash.get("socket_state_before", {}).get(name, {})
    stale = crash.get("socket_state_after_kill", {}).get(name, {})
    rebound = after_restart.get(name, {})
    if proof_required:
        if not (before.get("absolute") and before.get("exists") and before.get("is_socket") and before.get("connectable")):
            errors.append(f"{name} endpoint was not a live absolute socket before SIGKILL")
        if not (stale.get("exists") and stale.get("is_socket") and not stale.get("connectable")):
            errors.append(f"{name} endpoint was not a stale, unreachable socket after SIGKILL")
        if stale.get("path") != before.get("path") or stale.get("inode") != before.get("inode"):
            errors.append(f"{name} stale inode did not preserve the killed listener identity")
        if not (rebound.get("absolute") and rebound.get("exists") and rebound.get("is_socket") and rebound.get("connectable")):
            errors.append(f"{name} endpoint was not rebound and reachable after restart")
        if rebound.get("path") != stale.get("path"):
            errors.append(f"{name} endpoint path changed across restart")
        if rebound.get("inode") == stale.get("inode"):
            errors.append(f"{name} endpoint retained the stale inode after restart")
    socket_rows[name] = {
        "path": rebound.get("path"),
        "inode_before_kill": before.get("inode"),
        "stale_inode_after_kill": stale.get("inode"),
        "inode_after_restart": rebound.get("inode"),
        "stale_socket_detected": bool(stale.get("exists") and stale.get("is_socket") and not stale.get("connectable")),
        "ready_after_restart": bool(rebound.get("exists") and rebound.get("is_socket") and rebound.get("connectable")),
        "stale_detected_at_ms": crash.get("socket_state_after_kill", {}).get("observed_at_ms"),
        "ready_at_ms": after_restart.get("observed_at_ms"),
    }
if errors:
    raise SystemExit("; ".join(errors))
value = {
    "kind": "daemon_process_restart",
    "status": "online",
    "old_pid": int(old_pid),
    "new_pid": new_pid,
    "new_process_command": process_command,
    "expected_daemon_path": expected,
    "state": status.get("connection", {}).get("state"),
    "state_code": status.get("connection", {}).get("state_code"),
    "session_admitted": status.get("product_presence", {}).get("session_admitted"),
    "ready_at_ms": int(ready_at_ms),
    "socket_recovery": {
        "proof_required": proof_required,
        "cleanup_owner": "daemon_listener_bind",
        "stale_socket_cleanup_explicit": proof_required,
        "manual_cleanup_required": False,
        "control": socket_rows["control"],
        "invocation": socket_rows["invocation"],
    },
}
pathlib.Path(path).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

if [[ "$INTERNAL_MODE" == kill ]]; then
  fixture_kill
  exit 0
elif [[ "$INTERNAL_MODE" == restart ]]; then
  fixture_restart
  exit 0
fi

mkdir -p "$OUT_DIR"
EVIDENCE_JSON="$OUT_DIR/evidence.json"
FIXTURE_DIR="$OUT_DIR/process-fixture"
VERIFIED_DIR="$OUT_DIR/browser-verifier"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"

merge_and_validate() {
  python3 - "$EVIDENCE_JSON" "$FIXTURE_DIR/crash.json" "$FIXTURE_DIR/restart.json" <<'PY'
import json
import pathlib
import sys

evidence_path, crash_path, restart_path = map(pathlib.Path, sys.argv[1:4])
evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
crash = json.loads(crash_path.read_text(encoding="utf-8"))
restart = json.loads(restart_path.read_text(encoding="utf-8"))
resume = evidence.get("transport_resume")
errors = []

def require(condition, message):
    if not condition:
        errors.append(message)

require(evidence.get("status") == "passed", "Browser evidence must pass")
require(isinstance(resume, dict), "transport_resume must be present")
require(crash.get("kind") == "daemon_process_sigkill", "crash kind must be daemon_process_sigkill")
require(crash.get("status") == "killed", "daemon SIGKILL must be observed")
require(crash.get("signal") == "SIGKILL", "crash signal must be SIGKILL")
require(restart.get("kind") == "daemon_process_restart", "restart kind must be daemon_process_restart")
require(restart.get("status") == "online", "restarted daemon must be online")
require(restart.get("state_code") == "J800", "restarted daemon must reach J800")
require(restart.get("session_admitted") is True, "restarted daemon session must be admitted")
require(isinstance(crash.get("old_pid"), int) and crash.get("old_pid") > 1,
        "crash old_pid must be recorded")
require(isinstance(restart.get("new_pid"), int) and restart.get("new_pid") > 1,
        "restart new_pid must be recorded")
require(crash.get("old_pid") != restart.get("new_pid"), "daemon PID must change after SIGKILL")
expected = restart.get("expected_daemon_path")
command = restart.get("new_process_command")
require(isinstance(expected, str) and expected != "", "restart expected daemon path must be recorded")
require(isinstance(command, str) and (command == expected or command.startswith(expected + " ")),
        "restarted PID must resolve to the expected daemon path")
socket_recovery = restart.get("socket_recovery")
require(isinstance(socket_recovery, dict), "socket recovery proof must be recorded")
if isinstance(socket_recovery, dict):
    require(socket_recovery.get("proof_required") is True,
            "live aggregate requires real socket proof")
    require(socket_recovery.get("cleanup_owner") == "daemon_listener_bind",
            "daemon listener bind must own stale socket cleanup")
    require(socket_recovery.get("stale_socket_cleanup_explicit") is True,
            "stale socket cleanup must be explicit")
    require(socket_recovery.get("manual_cleanup_required") is False,
            "manual stale socket cleanup must not be required")
    for name in ("control", "invocation"):
        row = socket_recovery.get(name)
        require(isinstance(row, dict), f"{name} socket recovery row must be recorded")
        if isinstance(row, dict):
            require(row.get("stale_socket_detected") is True,
                    f"{name} stale socket must be detected after SIGKILL")
            require(row.get("ready_after_restart") is True,
                    f"{name} socket must be reachable after restart")
            require(isinstance(row.get("stale_inode_after_kill"), int),
                    f"{name} stale inode must be recorded")
            require(isinstance(row.get("inode_after_restart"), int)
                    and row.get("inode_after_restart") != row.get("stale_inode_after_kill"),
                    f"{name} listener must replace the stale inode")
require(isinstance(resume, dict) and resume.get("same_public_session") is True,
        "same public RemoteApp session must recover")
require(isinstance(resume, dict) and resume.get("new_peer_connection") is True,
        "replacement PeerConnection must connect")
require(isinstance(resume, dict) and resume.get("transport_epoch_increased") is True,
        "Runtime transport epoch must increase")
require(isinstance(resume, dict) and resume.get("watch_events_reestablished") is True,
        "watch_events must reattach")
require(isinstance(resume, dict) and int(resume.get("frames_presented_after_resume", 0)) > 0,
        "media must render after crash recovery")
states = [row.get("state_code") for row in evidence.get("device_state_snapshots", []) if isinstance(row, dict)]
require("C440" in states and states[-1:] == ["J700"],
        "Browser must observe Device offline then online")
if errors:
    raise SystemExit("; ".join(errors))

evidence["daemon_crash_recovery"] = {
    "proof_mode": "real_active_session_daemon_sigkill",
    "scenario": "daemon_restart_active_session",
    "crash": crash,
    "restart": restart,
    "same_public_session": True,
    "session_id": evidence.get("session_id"),
    "selected_resource_ura": evidence.get("selected_resource_ura"),
    "transport_resume": resume,
    "stale_socket_restart_cleanup": restart.get("socket_recovery"),
}
evidence_path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

write_report() {
  local status="$1"
  local reason="$2"
  python3 - "$REPORT_JSON" "$REPORT_MD" "$status" "$reason" "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

report_path, md_path, status, reason, evidence_path = sys.argv[1:6]
report = {
    "script": "tools/scripts/host-remoteapp-daemon-sigkill-e2e.sh",
    "status": status,
    "reason": reason,
    "scenario": "daemon_restart_active_session",
    "evidence_json": evidence_path,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# RemoteApp active-session daemon SIGKILL E2E\n\n"
    f"- Status: `{status}`\n"
    f"- Reason: `{reason}`\n"
    f"- Evidence: `{evidence_path}`\n",
    encoding="utf-8",
)
PY
}

if [[ "$MODE" == skip ]]; then
  write_report skipped "opt in with --run"
  echo "[host-remoteapp-daemon-sigkill-e2e] SKIP: $REPORT_MD"
  exit 0
fi

if [[ "$MODE" == self-test ]]; then
  cat >"$EVIDENCE_JSON" <<'JSON'
{"status":"passed","session_id":"rd-self","selected_resource_ura":"easynet:///r/test/resource/device.dev/streams/window.1","transport_resume":{"same_public_session":true,"new_peer_connection":true,"transport_epoch_increased":true,"watch_events_reestablished":true,"frames_presented_after_resume":1},"device_state_snapshots":[{"state_code":"J700"},{"state_code":"C440"},{"state_code":"J700"}]}
JSON
  mkdir -p "$FIXTURE_DIR"
  cat >"$FIXTURE_DIR/crash.json" <<'JSON'
{"kind":"daemon_process_sigkill","status":"killed","signal":"SIGKILL","old_pid":1001,"process_command":"/tmp/easynet-daemon","expected_daemon_path":"/tmp/easynet-daemon","signal_at_ms":1,"observed_dead_at_ms":2,"socket_state_before":{"observed_at_ms":1,"control":{"path":"/tmp/control.sock","absolute":true,"exists":true,"is_socket":true,"inode":101,"connectable":true},"invocation":{"path":"/tmp/daemon.sock","absolute":true,"exists":true,"is_socket":true,"inode":201,"connectable":true}},"socket_state_after_kill":{"observed_at_ms":2,"control":{"path":"/tmp/control.sock","absolute":true,"exists":true,"is_socket":true,"inode":101,"connectable":false},"invocation":{"path":"/tmp/daemon.sock","absolute":true,"exists":true,"is_socket":true,"inode":201,"connectable":false}}}
JSON
  cat >"$FIXTURE_DIR/restart.json" <<'JSON'
{"kind":"daemon_process_restart","status":"online","old_pid":1001,"new_pid":1002,"new_process_command":"/tmp/easynet-daemon","expected_daemon_path":"/tmp/easynet-daemon","state":"FRONTEND_CONNECTED","state_code":"J800","session_admitted":true,"ready_at_ms":3,"socket_recovery":{"proof_required":true,"cleanup_owner":"daemon_listener_bind","stale_socket_cleanup_explicit":true,"manual_cleanup_required":false,"control":{"path":"/tmp/control.sock","inode_before_kill":101,"stale_inode_after_kill":101,"inode_after_restart":102,"stale_socket_detected":true,"ready_after_restart":true,"stale_detected_at_ms":2,"ready_at_ms":3},"invocation":{"path":"/tmp/daemon.sock","inode_before_kill":201,"stale_inode_after_kill":201,"inode_after_restart":202,"stale_socket_detected":true,"ready_after_restart":true,"stale_detected_at_ms":2,"ready_at_ms":3}}}
JSON
  merge_and_validate
  write_report passed "contract self-test only"
  echo "[host-remoteapp-daemon-sigkill-e2e] SELF-TEST PASS: $REPORT_MD"
  exit 0
fi

for name in \
  EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL \
  EASYNET_REMOTEAPP_BROWSER_DEVICE_ID \
  EASYNET_REMOTEAPP_BROWSER_EMAIL \
  EASYNET_REMOTEAPP_BROWSER_PASSWORD
do
  [[ -n "${!name:-}" ]] || { echo "$name is required" >&2; exit 64; }
done
[[ -x "$EASYNET_BIN" ]] || { echo "missing easynet binary: $EASYNET_BIN" >&2; exit 69; }
[[ -f "$RUNNER" ]] || { echo "missing Browser runner: $RUNNER" >&2; exit 69; }

mkdir -p "$FIXTURE_DIR"
disconnect_command="$(python3 - "$0" "$FIXTURE_DIR" <<'PY'
import shlex, sys
print(f"{shlex.quote(sys.argv[1])} --fixture-kill {shlex.quote(sys.argv[2])}")
PY
)"
reconnect_command="$(python3 - "$0" "$FIXTURE_DIR" <<'PY'
import shlex, sys
print(f"{shlex.quote(sys.argv[1])} --fixture-restart {shlex.quote(sys.argv[2])}")
PY
)"

EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON="$EVIDENCE_JSON" \
EASYNET_REMOTEAPP_BROWSER_TARGET_KIND="$TARGET_KIND" \
EASYNET_REMOTEAPP_BROWSER_RESUME_DISCONNECT_COMMAND="$disconnect_command" \
EASYNET_REMOTEAPP_BROWSER_RESUME_RECONNECT_COMMAND="$reconnect_command" \
node "$RUNNER"

merge_and_validate
"$BROWSER_VERIFIER" --run --evidence-json "$EVIDENCE_JSON" --out-dir "$VERIFIED_DIR" >/dev/null
write_report passed "active Browser session recovered after exact-PID daemon SIGKILL"
echo "[host-remoteapp-daemon-sigkill-e2e] PASS: $REPORT_MD"
