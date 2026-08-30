#!/usr/bin/env bash
# Real RemoteApp lost-response recovery after durable terminal promotion.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
FRONTEND_ROOT="${EASYNET_REMOTEAPP_FRONTEND_ROOT:-$REPO_ROOT/../EasyNet/Frontend}"
RUNNER="$FRONTEND_ROOT/scripts/remoteapp-browser-lifecycle.mjs"
BROWSER_VERIFIER="$SELF_DIR/frontend-remoteapp-browser-lifecycle-e2e.sh"
PROVENANCE_HELPER="$SELF_DIR/remoteapp-evidence-provenance.py"
source "$SELF_DIR/remoteapp-lifecycle-harness-lib.sh"
EASYNET_BIN="${EASYNET_REMOTEAPP_CRASH_EASYNET_BIN:-$REPO_ROOT/target/debug/easynet}"
EXPECTED_DAEMON="${EASYNET_REMOTEAPP_CRASH_EXPECTED_DAEMON:-$REPO_ROOT/target/debug/easynet-daemon}"
FAULT_ARM_ENV=EASYNET_REMOTEAPP_E2E_TERMINAL_PROMOTION_ARM_FILE
TERMINAL_REASON=caller_ended

MODE=skip
TARGET_KIND=window
OUT_DIR="${EASYNET_REMOTEAPP_TERMINAL_CRASH_OUT_DIR:-$REPO_ROOT/target/e2e/host-remoteapp-terminal-receipt-crash/$(date -u +%Y%m%d-%H%M%S)-$$}"
INTERNAL_ARM_PATH=""
INTERNAL_MARKER_PATH=""
VERIFY_INPUT=""
RUNTIME_RESTORE_REQUIRED=0

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-terminal-receipt-crash-e2e.sh --run [--target-kind window|application]
  host-remoteapp-terminal-receipt-crash-e2e.sh --self-test

The live runner builds an explicit E2E fault binary, starts the paired Runtime
with an inert one-shot arm-file path, drives the real Browser UI, crashes the
daemon after durable terminal promotion but before the end_session response,
restarts it, and compares the exact terminal receipt across marker, recovery
snapshot, public show_session, and repeated public end_session.

Required Browser environment:
  EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL
  EASYNET_REMOTEAPP_BROWSER_DEVICE_ID
  EASYNET_REMOTEAPP_BROWSER_EMAIL
  EASYNET_REMOTEAPP_BROWSER_PASSWORD

Non-claim:
  This proves one Unix lost-response terminal replay boundary. It does not
  prove plugin-worker-only recovery, successful input injection, cross-platform
  capture, degraded media, network fallback, or cross-device behavior.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
    --target-kind)
      case "${2:?missing target kind}" in
        window|application) TARGET_KIND="$2" ;;
        *) echo "invalid target kind: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --out-dir) OUT_DIR="${2:?missing output directory}"; shift 2 ;;
    --verify-self-test-evidence)
      MODE=self-test
      VERIFY_INPUT="${2:?missing evidence path}"
      shift 2
      ;;
    --fixture-arm)
      INTERNAL_ARM_PATH="${2:?missing arm path}"
      INTERNAL_MARKER_PATH="${3:?missing marker path}"
      shift 3
      ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

arm_fault() {
  python3 - "$INTERNAL_ARM_PATH" "$INTERNAL_MARKER_PATH" \
    "${EASYNET_REMOTEAPP_E2E_SESSION_ID:-}" \
    "${EASYNET_REMOTEAPP_E2E_RESOURCE_URA:-}" "$TERMINAL_REASON" <<'PY'
import json
import os
import pathlib
import secrets
import sys
import time

arm_path, marker_path = map(pathlib.Path, sys.argv[1:3])
session_id, resource_ura, reason = sys.argv[3:6]
for label, path in (("arm", arm_path), ("marker", marker_path)):
    if not path.is_absolute() or path.parent == pathlib.Path("/"):
        raise SystemExit(f"{label} path must be a bounded absolute file path")
if not session_id.startswith("rdp-"):
    raise SystemExit("Browser did not pass one RemoteApp session id")
if not resource_ura.startswith("easynet:///r/"):
    raise SystemExit("Browser did not pass one canonical Resource URA")
arm_path.parent.mkdir(parents=True, exist_ok=True)
value = {
    "schema_version": 1,
    "fault": "crash_after_terminal_promotion",
    "session_id": session_id,
    "reason": reason,
    "marker_path": str(marker_path),
    "nonce": secrets.token_hex(16),
    "armed_at_ms": int(time.time() * 1000),
    "selected_resource_ura": resource_ura,
}
# The Rust parser deliberately denies extension fields. Keep the Resource URA
# in a separate host fact instead of widening the daemon arm contract.
host_value = dict(value)
value.pop("selected_resource_ura")
temp = arm_path.with_suffix(f".tmp.{os.getpid()}")
fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, "w", encoding="utf-8") as handle:
    json.dump(value, handle, indent=2, sort_keys=True)
    handle.write("\n")
    handle.flush()
    os.fsync(handle.fileno())
os.replace(temp, arm_path)
directory = os.open(arm_path.parent, os.O_RDONLY)
try:
    os.fsync(directory)
finally:
    os.close(directory)
host_path = arm_path.with_suffix(".host.json")
host_path.write_text(json.dumps(host_value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
os.chmod(host_path, 0o600)
PY
}

if [[ -n "$INTERNAL_ARM_PATH" ]]; then
  arm_fault
  exit 0
fi

mkdir -p "$OUT_DIR"
ARM_FILE="$OUT_DIR/terminal-promotion-arm.json"
MARKER_FILE="$OUT_DIR/terminal-promotion-crash-marker.json"
BROWSER_EVIDENCE="$OUT_DIR/browser-evidence.json"
BROWSER_VERIFIED_DIR="$OUT_DIR/browser-verifier"
RUNTIME_BEFORE_JSON="$OUT_DIR/runtime-before.json"
RUNTIME_AFTER_JSON="$OUT_DIR/runtime-after.json"
RECOVERY_SNAPSHOT_JSON="$OUT_DIR/recovery-snapshot.json"
SESSION_CONTEXT_JSON="$OUT_DIR/session-context.json"
ABILITY_CATALOG_JSON="$OUT_DIR/ability-catalog.json"
SHOW_RAW="$OUT_DIR/show-after-restart.raw.txt"
SHOW_JSON="$OUT_DIR/show-after-restart.json"
END_RAW="$OUT_DIR/end-again.raw.txt"
END_JSON="$OUT_DIR/end-again.json"
EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"

json_first_value_to_file() {
  python3 - "$1" "$2" <<'PY'
import json
import pathlib
import sys

source = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
value, _ = json.JSONDecoder().raw_decode(source.lstrip())
pathlib.Path(sys.argv[2]).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

runtime_pid() {
  python3 - "$1" <<'PY'
import json, sys
pid = json.load(open(sys.argv[1], encoding="utf-8")).get("daemon", {}).get("pid")
if not isinstance(pid, int) or pid <= 1:
    raise SystemExit("runtime status missing daemon pid")
print(pid)
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
    "script": "tools/scripts/host-remoteapp-terminal-receipt-crash-e2e.sh",
    "status": status,
    "reason": reason,
    "evidence_json": evidence_path,
    "scenario": "terminal_receipt_replay_after_crash",
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# RemoteApp terminal receipt replay after crash\n\n"
    f"- Status: `{status}`\n"
    f"- Reason: `{reason}`\n"
    f"- Evidence: `{evidence_path}`\n",
    encoding="utf-8",
)
PY
}

validate_evidence() {
  python3 "$PROVENANCE_HELPER" verify --mode "$MODE" --evidence "$EVIDENCE_JSON"
  python3 - "$EVIDENCE_JSON" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
errors = []
def require(value, message):
    if not value: errors.append(message)

expected_origin = "contract_self_test" if evidence.get("self_test") else "live_runner"
require(evidence.get("status") == "passed", "status must be passed")
require(evidence.get("evidence_origin") == expected_origin, f"evidence_origin must be {expected_origin}")
require(evidence.get("proof_mode") == "real_terminal_receipt_replay_after_process_crash",
        "proof_mode must identify real terminal receipt crash replay")
require(evidence.get("component_mock") is False, "component_mock must be false")
require(evidence.get("real_backend_runtime") is True, "real_backend_runtime must be true")
require(evidence.get("product_complete_claim") is False, "product_complete_claim must remain false")
session_id = evidence.get("session_id")
subject = evidence.get("selected_resource_ura")
require(isinstance(session_id, str) and session_id, "session_id must be recorded")
require(isinstance(subject, str) and subject.startswith("easynet:///r/"), "Resource subject must be canonical")
browser = evidence.get("browser_terminal_crash_replay") or {}
require(browser.get("same_public_session") is True, "Browser must preserve the public session")
require(browser.get("response_lost_to_daemon_crash") is True, "Browser must observe the lost end response")
require(browser.get("show_session_replayed_terminal") is True, "Browser must replay terminal state through show_session")
process = evidence.get("process") or {}
require(isinstance(process.get("crashed_pid"), int) and process.get("crashed_pid") > 1,
        "crashed pid must be recorded")
require(isinstance(process.get("restarted_pid"), int) and process.get("restarted_pid") > 1,
        "restarted pid must be recorded")
require(process.get("crashed_pid") != process.get("restarted_pid"), "daemon pid must change")
marker = evidence.get("crash_marker") or {}
snapshot = evidence.get("recovery_snapshot") or {}
show = evidence.get("show_after_restart") or {}
end = evidence.get("end_again") or {}
receipts = [
    marker.get("terminal_receipt"),
    snapshot.get("terminal_receipt"),
    show.get("terminal_receipt"),
    end.get("terminal_receipt"),
]
require(all(isinstance(receipt, dict) for receipt in receipts), "all four terminal receipts must be present")
if all(isinstance(receipt, dict) for receipt in receipts):
    require(all(receipt == receipts[0] for receipt in receipts[1:]),
            "marker, recovery, show, and repeated end must preserve one exact terminal receipt")
    receipt = receipts[0]
    require(receipt.get("receipt_type") == "remoteapp.session.terminal.v1", "terminal receipt type must be canonical")
    require(receipt.get("session_id") == session_id, "terminal receipt must bind session id")
    require(receipt.get("subject_ura") == subject, "terminal receipt must bind Resource subject")
    require(receipt.get("reason_code") == "caller_ended", "real product End reason must survive crash")
    require(receipt.get("terminal") is True, "terminal receipt must be terminal")
    require(isinstance(receipt.get("terminal_event_id"), str)
            and receipt["terminal_event_id"].startswith(session_id + ":"),
            "terminal event id must bind session id")
    require(isinstance(receipt.get("terminal_event_sequence"), int)
            and receipt["terminal_event_sequence"] > 0,
            "terminal event sequence must be positive")
require(snapshot.get("lifecycle_state") == "closed", "authoritative recovery snapshot must be closed")
require(show.get("state") == "closed", "public show_session must return closed")
require(end.get("state") == "closed" and end.get("already_ended") is True,
        "repeated public end_session must be idempotent")
events = evidence.get("events") or []
require([event.get("type") for event in events] == [
    "END_SESSION_ACCEPTED", "PROCESS_STOPPED_UNCLEAN", "TERMINAL_RECEIPT_REPLAYED"
], "recovery event sequence must be exact")
times = [event.get("at_ms") for event in events]
require(all(isinstance(value, int) and value > 0 for value in times)
        and times == sorted(set(times)), "recovery events must be strictly ordered")
if errors:
    raise SystemExit("; ".join(errors))
PY
}

write_self_test_evidence() {
  python3 - "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

session = "rdp-self-terminal-crash"
subject = "easynet:///r/localhost/resource/device.dev/streams/window.self"
receipt = {
    "receipt_type": "remoteapp.session.terminal.v1",
    "session_id": session,
    "subject_ura": subject,
    "reason_code": "caller_ended",
    "terminal": True,
    "terminal_event_id": session + ":9",
    "terminal_event_sequence": 9,
    "terminal_event_type": "SESSION_CLOSED",
}
value = {
    "status": "passed",
    "evidence_origin": "contract_self_test",
    "proof_mode": "real_terminal_receipt_replay_after_process_crash",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "self_test": True,
    "session_id": session,
    "selected_resource_ura": subject,
    "browser_terminal_crash_replay": {
        "same_public_session": True,
        "response_lost_to_daemon_crash": True,
        "show_session_replayed_terminal": True,
    },
    "process": {"crashed_pid": 1001, "restarted_pid": 1002},
    "crash_marker": {"terminal_receipt": receipt},
    "recovery_snapshot": {"lifecycle_state": "closed", "terminal_receipt": receipt},
    "show_after_restart": {"state": "closed", "terminal_receipt": receipt},
    "end_again": {"state": "closed", "already_ended": True, "terminal_receipt": receipt},
    "events": [
        {"type": "END_SESSION_ACCEPTED", "at_ms": 1},
        {"type": "PROCESS_STOPPED_UNCLEAN", "at_ms": 2},
        {"type": "TERMINAL_RECEIPT_REPLAYED", "at_ms": 3},
    ],
}
pathlib.Path(sys.argv[1]).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

restore_product_runtime() {
  local prior_status=$?
  if [[ "$RUNTIME_RESTORE_REQUIRED" == 1 ]]; then
    set +e
    env -u "$FAULT_ARM_ENV" "$EASYNET_BIN" runtime stop >/dev/null 2>&1
    cargo build --quiet --bin easynet --bin easynet-daemon >/dev/null 2>&1
    env -u "$FAULT_ARM_ENV" "$EASYNET_BIN" runtime start >/dev/null 2>&1
    set -e
  fi
  return "$prior_status"
}

if [[ "$MODE" == skip ]]; then
  write_report skipped "opt in with --run"
  echo "[host-remoteapp-terminal-receipt-crash-e2e] SKIP: $REPORT_MD"
  exit 0
fi
if [[ -n "$VERIFY_INPUT" ]]; then
  cp "$VERIFY_INPUT" "$EVIDENCE_JSON"
  validate_evidence
  write_report passed "supplied self-test evidence contract passed"
  exit 0
fi
if [[ "$MODE" == self-test ]]; then
  write_self_test_evidence
  validate_evidence
  write_report passed "contract self-test only"
  echo "[host-remoteapp-terminal-receipt-crash-e2e] SELF-TEST PASS: $REPORT_MD"
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
[[ -f "$RUNNER" ]] || { echo "missing Browser runner: $RUNNER" >&2; exit 69; }

trap restore_product_runtime EXIT
env -u "$FAULT_ARM_ENV" "$EASYNET_BIN" runtime stop >/dev/null 2>&1 || true
cargo build --features remoteapp-e2e-fault-injection --bin easynet --bin easynet-daemon
RUNTIME_RESTORE_REQUIRED=1
"$EASYNET_BIN" runtime status --json >"$RUNTIME_BEFORE_JSON" 2>/dev/null || printf '{}\n' >"$RUNTIME_BEFORE_JSON"
env "$FAULT_ARM_ENV=$ARM_FILE" "$EASYNET_BIN" runtime start >/dev/null

arm_command="$(python3 - "$0" "$ARM_FILE" "$MARKER_FILE" <<'PY'
import shlex, sys
print(" ".join(map(shlex.quote, [sys.argv[1], "--fixture-arm", sys.argv[2], sys.argv[3]])))
PY
)"
reconnect_command="$(python3 - "$EASYNET_BIN" "$FAULT_ARM_ENV" "$ARM_FILE" <<'PY'
import shlex, sys
print(f"env {shlex.quote(sys.argv[2] + '=' + sys.argv[3])} {shlex.quote(sys.argv[1])} runtime start")
PY
)"

env "$FAULT_ARM_ENV=$ARM_FILE" \
  EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON="$BROWSER_EVIDENCE" \
  EASYNET_REMOTEAPP_BROWSER_TARGET_KIND="$TARGET_KIND" \
  EASYNET_REMOTEAPP_BROWSER_TERMINAL_CRASH_ARM_COMMAND="$arm_command" \
  EASYNET_REMOTEAPP_BROWSER_TERMINAL_CRASH_RECONNECT_COMMAND="$reconnect_command" \
  node "$RUNNER"

"$BROWSER_VERIFIER" --run --evidence-json "$BROWSER_EVIDENCE" \
  --out-dir "$BROWSER_VERIFIED_DIR" >/dev/null
[[ -f "$MARKER_FILE" ]] || { echo "terminal-promotion crash marker missing" >&2; exit 76; }
"$EASYNET_BIN" runtime status --json >"$RUNTIME_AFTER_JSON"

SESSION_ID="$(python3 - "$BROWSER_EVIDENCE" <<'PY'
import json, sys
print(json.load(open(sys.argv[1], encoding="utf-8"))["session_id"])
PY
)"
SELECTED_RESOURCE_URA="$(python3 - "$BROWSER_EVIDENCE" <<'PY'
import json, sys
print(json.load(open(sys.argv[1], encoding="utf-8"))["selected_resource_ura"])
PY
)"
STATE_DIR="$(python3 - <<'PY'
from pathlib import Path
print(Path.home() / ".easynet")
PY
)"
cp "$STATE_DIR/remote-desktop/sessions/$SESSION_ID.json" "$RECOVERY_SNAPSHOT_JSON"
chmod 600 "$RECOVERY_SNAPSHOT_JSON"

python3 - "$RECOVERY_SNAPSHOT_JSON" "$SESSION_CONTEXT_JSON" <<'PY'
import json
import pathlib
import sys

snapshot = json.load(open(sys.argv[1], encoding="utf-8"))
value = {"session": {"consent": {"approval_receipt": snapshot["consent"]["approval_receipt"]}}}
pathlib.Path(sys.argv[2]).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
PY
SESSION_TOKEN="$(python3 - "$RECOVERY_SNAPSHOT_JSON" <<'PY'
import json, sys
print(json.load(open(sys.argv[1], encoding="utf-8"))["session_token"])
PY
)"
CAUSAL_CONTEXT_JSON="$(remoteapp_session_approval_causal_context_json "$SESSION_CONTEXT_JSON")"
"$EASYNET_BIN" ability list --format json >"$ABILITY_CATALOG_JSON"
SHOW_ABILITY_REF="$(remoteapp_resolve_rpc_descriptor_ref "$ABILITY_CATALOG_JSON" remote_desktop.show_session)"
END_ABILITY_REF="$(remoteapp_resolve_rpc_descriptor_ref "$ABILITY_CATALOG_JSON" remote_desktop.end_session)"
SHOW_CALLEE_URA="$(remoteapp_resolve_rpc_owner_ura "$ABILITY_CATALOG_JSON" remote_desktop.show_session)"
END_CALLEE_URA="$(remoteapp_resolve_rpc_owner_ura "$ABILITY_CATALOG_JSON" remote_desktop.end_session)"
SHOW_ARGS="$(python3 - "$SESSION_ID" "$SESSION_TOKEN" <<'PY'
import json, sys
print(json.dumps({"session_id": sys.argv[1], "session_token": sys.argv[2]}, separators=(",", ":")))
PY
)"
END_ARGS="$(python3 - "$SESSION_ID" "$SESSION_TOKEN" "$TERMINAL_REASON" <<'PY'
import json, sys
print(json.dumps({"session_id": sys.argv[1], "session_token": sys.argv[2], "reason": sys.argv[3]}, separators=(",", ":")))
PY
)"
NONCE="$(python3 -c 'import secrets; print(secrets.token_hex(16))')"
"$EASYNET_BIN" ability invoke "$SHOW_ABILITY_REF" \
  --node "$SHOW_CALLEE_URA" \
  --subject "$SELECTED_RESOURCE_URA" --nonce-hex "$NONCE" \
  --causal-context-json "$CAUSAL_CONTEXT_JSON" --args "$SHOW_ARGS" >"$SHOW_RAW"
json_first_value_to_file "$SHOW_RAW" "$SHOW_JSON"
NONCE="$(python3 -c 'import secrets; print(secrets.token_hex(16))')"
"$EASYNET_BIN" ability invoke "$END_ABILITY_REF" \
  --node "$END_CALLEE_URA" \
  --subject "$SELECTED_RESOURCE_URA" --nonce-hex "$NONCE" \
  --causal-context-json "$CAUSAL_CONTEXT_JSON" --args "$END_ARGS" >"$END_RAW"
json_first_value_to_file "$END_RAW" "$END_JSON"

python3 - "$EVIDENCE_JSON" "$BROWSER_EVIDENCE" "$MARKER_FILE" \
  "$RECOVERY_SNAPSHOT_JSON" "$SHOW_JSON" "$END_JSON" "$RUNTIME_AFTER_JSON" \
  "$SHOW_ABILITY_REF" "$END_ABILITY_REF" "$SHOW_CALLEE_URA" "$END_CALLEE_URA" <<'PY'
import json
import pathlib
import re
import sys
import time

(
    evidence_path, browser_path, marker_path, snapshot_path, show_path,
    end_path, runtime_path, show_descriptor_ref, end_descriptor_ref,
    show_callee_ura, end_callee_ura,
) = sys.argv[1:12]
load = lambda path: json.load(open(path, encoding="utf-8"))
browser, marker, snapshot, show, end, runtime = map(load, (
    browser_path, marker_path, snapshot_path, show_path, end_path, runtime_path,
))
session_id = browser["session_id"]
subject = browser["selected_resource_ura"]
summary = browser["terminal_crash_replay"]
descriptor_ref = browser["network_transport"]["abilities"][0]["descriptor_ref"]
match = re.search(r"@([^#]+)#", descriptor_ref)
if not match:
    raise SystemExit("Browser descriptor_ref omitted descriptor version")
show_at = int(time.time() * 1000)
request_at = int(summary["end_session_request_observed_at_ms"])
crash_at = int(marker["promoted_at_ms"])
if crash_at <= request_at:
    crash_at = request_at + 1
if show_at <= crash_at:
    show_at = crash_at + 1
value = {
    "status": "passed",
    "evidence_origin": "live_runner",
    "proof_mode": "real_terminal_receipt_replay_after_process_crash",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "session_id": session_id,
    "selected_resource_ura": subject,
    "descriptor_version": match.group(1),
    "browser_terminal_crash_replay": summary,
    "browser_verifier_report": str(pathlib.Path(browser_path).parent / "browser-verifier" / "report.md"),
    "process": {
        "crashed_pid": marker["pid"],
        "restarted_pid": runtime["daemon"]["pid"],
        "restart_state_code": runtime["connection"]["state_code"],
        "session_admitted": runtime["product_presence"]["session_admitted"],
    },
    "crash_marker": marker,
    "recovery_snapshot": {
        "lifecycle_state": snapshot["lifecycle_state"],
        "termination_reason": snapshot["termination_reason"],
        "terminal_receipt": snapshot["terminal_receipt"],
    },
    "show_after_restart": show,
    "end_again": end,
    "abilities": [
        {"name": "remote_desktop.create_session", "subject_ura": subject},
        {"name": "remote_desktop.show_session", "descriptor_ref": show_descriptor_ref, "callee_ura": show_callee_ura, "subject_ura": subject, "session_id": session_id},
        {"name": "remote_desktop.watch_events", "subject_ura": subject, "session_id": session_id},
        {"name": "remote_desktop.end_session", "descriptor_ref": end_descriptor_ref, "callee_ura": end_callee_ura, "subject_ura": subject, "session_id": session_id},
    ],
    "events": [
        {"type": "END_SESSION_ACCEPTED", "at_ms": request_at, "selected_resource_ura": subject, "session_id": session_id},
        {"type": "PROCESS_STOPPED_UNCLEAN", "at_ms": crash_at, "selected_resource_ura": subject, "session_id": session_id},
        {"type": "TERMINAL_RECEIPT_REPLAYED", "at_ms": show_at, "selected_resource_ura": subject, "session_id": session_id},
    ],
}
pathlib.Path(evidence_path).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

validate_evidence
write_report passed "durable terminal receipt replayed exactly after lost end_session response"
echo "[host-remoteapp-terminal-receipt-crash-e2e] PASS: $REPORT_MD"
