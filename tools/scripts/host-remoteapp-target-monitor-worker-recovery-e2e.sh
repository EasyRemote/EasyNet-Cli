#!/usr/bin/env bash
# Real RemoteApp target-monitor worker-only crash/recovery proof.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
FRONTEND_ROOT="${EASYNET_REMOTEAPP_FRONTEND_ROOT:-$REPO_ROOT/../EasyNet/Frontend}"
RUNNER="$FRONTEND_ROOT/scripts/remoteapp-browser-lifecycle.mjs"
BROWSER_VERIFIER="$SELF_DIR/frontend-remoteapp-browser-lifecycle-e2e.sh"
PROVENANCE_HELPER="$SELF_DIR/remoteapp-evidence-provenance.py"
EASYNET_BIN="${EASYNET_REMOTEAPP_WORKER_EASYNET_BIN:-$REPO_ROOT/target/debug/easynet}"
FAULT_ARM_ENV=EASYNET_REMOTEAPP_E2E_TARGET_MONITOR_ARM_FILE

MODE=skip
TARGET_KIND=window
OUT_DIR="${EASYNET_REMOTEAPP_WORKER_RECOVERY_OUT_DIR:-$REPO_ROOT/target/e2e/host-remoteapp-target-monitor-worker-recovery/$(date -u +%Y%m%d-%H%M%S)-$$}"
INTERNAL_ARM_PATH=""
INTERNAL_MARKER_PATH=""
VERIFY_INPUT=""
RUNTIME_RESTORE_REQUIRED=0

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-target-monitor-worker-recovery-e2e.sh --run [--target-kind window|application]
  host-remoteapp-target-monitor-worker-recovery-e2e.sh --self-test

The live runner builds the explicitly feature-gated E2E fault binary, starts
the paired Runtime with an inert owner-only arm path, drives the real Browser
UI, crashes only the target-monitor generation, and proves that the daemon,
session, consent, target binding, WebRTC transport, and media source remain
stable while a replacement generation completes a successful host poll.

Required Browser environment:
  EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL
  EASYNET_REMOTEAPP_BROWSER_DEVICE_ID
  EASYNET_REMOTEAPP_BROWSER_EMAIL
  EASYNET_REMOTEAPP_BROWSER_PASSWORD

Non-claim:
  This proves one macOS worker-only recovery boundary. It does not prove daemon
  crash recovery, Windows/Linux capture or input, degraded media, network
  fallback, cross-device behavior, or overall product completion.
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
    "${EASYNET_REMOTEAPP_E2E_RESOURCE_URA:-}" <<'PY'
import json
import os
import pathlib
import secrets
import sys
import time

arm_path, marker_path = map(pathlib.Path, sys.argv[1:3])
session_id, resource_ura = sys.argv[3:5]
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
    "fault": "crash_target_monitor_generation",
    "session_id": session_id,
    "marker_path": str(marker_path),
    "nonce": secrets.token_hex(16),
    "armed_at_ms": int(time.time() * 1000),
}
temporary = arm_path.with_suffix(f".tmp.{os.getpid()}")
fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, "w", encoding="utf-8") as handle:
    json.dump(value, handle, indent=2, sort_keys=True)
    handle.write("\n")
    handle.flush()
    os.fsync(handle.fileno())
os.replace(temporary, arm_path)
directory = os.open(arm_path.parent, os.O_RDONLY)
try:
    os.fsync(directory)
finally:
    os.close(directory)
host_value = dict(value)
host_value["selected_resource_ura"] = resource_ura
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
ARM_FILE="$OUT_DIR/target-monitor-arm.json"
MARKER_FILE="$OUT_DIR/target-monitor-crash-marker.json"
BROWSER_EVIDENCE="$OUT_DIR/browser-evidence.json"
BROWSER_VERIFIED_DIR="$OUT_DIR/browser-verifier"
RUNTIME_BEFORE_JSON="$OUT_DIR/runtime-before.json"
RUNTIME_AFTER_JSON="$OUT_DIR/runtime-after.json"
RECOVERY_SNAPSHOT_JSON="$OUT_DIR/recovery-snapshot.json"
EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"

write_report() {
  local status="$1"
  local reason="$2"
  python3 - "$REPORT_JSON" "$REPORT_MD" "$status" "$reason" "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

report_path, md_path, status, reason, evidence_path = sys.argv[1:6]
report = {
    "script": "tools/scripts/host-remoteapp-target-monitor-worker-recovery-e2e.sh",
    "status": status,
    "reason": reason,
    "evidence_json": evidence_path,
    "scenario": "target_monitor_worker_only_recovery",
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# RemoteApp target-monitor worker-only recovery\n\n"
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
    if not value:
        errors.append(message)

expected_origin = "contract_self_test" if evidence.get("self_test") else "live_runner"
require(evidence.get("status") == "passed", "status must be passed")
require(evidence.get("evidence_origin") == expected_origin,
        f"evidence_origin must be {expected_origin}")
require(evidence.get("proof_mode") == "real_target_monitor_worker_only_recovery",
        "proof_mode must identify target-monitor worker-only recovery")
require(evidence.get("component_mock") is False, "component_mock must be false")
require(evidence.get("real_backend_runtime") is True, "real_backend_runtime must be true")
require(evidence.get("product_complete_claim") is False,
        "product_complete_claim must remain false")
session_id = evidence.get("session_id")
subject = evidence.get("selected_resource_ura")
require(isinstance(session_id, str) and session_id.startswith("rdp-"),
        "session_id must be one RemoteApp session")
require(isinstance(subject, str) and subject.startswith("easynet:///r/"),
        "selected Resource URA must be canonical")

process = evidence.get("process") or {}
before_pid = process.get("before_pid")
after_pid = process.get("after_pid")
require(isinstance(before_pid, int) and before_pid > 1, "before pid must be recorded")
require(after_pid == before_pid, "worker-only recovery must preserve daemon pid")
require(process.get("before_state_code") == "J800" and process.get("after_state_code") == "J800",
        "Runtime must remain J800 across worker-only recovery")

marker = evidence.get("crash_marker") or {}
failed_generation = marker.get("generation")
require(marker.get("fault") == "crash_target_monitor_generation",
        "marker must identify target-monitor generation crash")
require(marker.get("session_id") == session_id, "marker must bind session id")
require(marker.get("pid") == before_pid, "marker pid must equal stable daemon pid")
require(isinstance(failed_generation, int) and failed_generation > 0,
        "marker generation must be positive")

browser = evidence.get("browser_worker_recovery") or {}
require(browser.get("proof_mode") == "real_browser_target_monitor_worker_recovery",
        "Browser must prove target-monitor worker recovery")
require(browser.get("session_id") == session_id and browser.get("subject_ura") == subject,
        "Browser recovery must bind session and Resource subject")
require(browser.get("same_public_session") is True,
        "Browser must preserve one public session")
for field in (
    "daemon_transport_epoch_preserved",
    "target_binding_epoch_preserved",
    "media_source_epoch_preserved",
    "consent_epoch_preserved",
):
    require(browser.get(field) is True, f"Browser {field} must be true")
for prefix in ("transport_epoch", "binding_epoch", "media_source_epoch", "consent_epoch"):
    before = browser.get(f"{prefix}_before")
    after = browser.get(f"{prefix}_after")
    require(isinstance(before, int) and before > 0 and after == before,
            f"Browser must preserve positive {prefix}")
require(isinstance(browser.get("frames_rendered_after_worker_restart"), int)
        and browser["frames_rendered_after_worker_restart"] > 0,
        "Browser must render a later frame")
require(browser.get("new_consent_required") is False,
        "worker recovery must not request new consent")

expected_types = [
    "PLUGIN_WORKER_CRASHED",
    "PLUGIN_WORKER_RESTARTED",
    "TARGET_MONITOR_RESTARTED",
]
for source in ("public_events", "persisted_events"):
    events = evidence.get(source) or []
    require([event.get("event_type") for event in events] == expected_types,
            f"{source} must contain exact ordered worker lifecycle events")
    if len(events) == 3 and isinstance(failed_generation, int):
        payloads = [event.get("payload") or {} for event in events]
        require(all(payload.get("component") == "target_monitor" for payload in payloads),
                f"{source} must bind target_monitor component")
        require(all(payload.get("failed_generation") == failed_generation for payload in payloads),
                f"{source} failed generation must match marker")
        restarted = payloads[1].get("restarted_generation")
        require(isinstance(restarted, int) and restarted > failed_generation,
                f"{source} restarted generation must increase")
        require(payloads[2].get("restarted_generation") == restarted,
                f"{source} functional recovery must bind replacement generation")
        sequences = [event.get("sequence") for event in events]
        require(all(isinstance(value, int) and value > 0 for value in sequences)
                and sequences == sorted(set(sequences)),
                f"{source} event sequences must be strictly ordered")

terminal = evidence.get("terminal_cleanup") or {}
require(terminal.get("completed") is True and terminal.get("session_id") == session_id,
        "same session must complete public terminal cleanup")
require(terminal.get("terminal") is True,
        "terminal cleanup must expose a terminal receipt")

if errors:
    raise SystemExit("; ".join(errors))
PY
}

write_self_test_evidence() {
  python3 - "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

session = "rdp-self-worker-recovery"
subject = "easynet:///r/localhost/resource/device.dev/streams/window.self"
failed, restarted = 7, 8
def event(sequence, event_type, **payload):
    return {
        "sequence": sequence,
        "event_type": event_type,
        "session_id": session,
        "subject_ura": subject,
        "payload": {"component": "target_monitor", "failed_generation": failed, **payload},
    }
events = [
    event(20, "PLUGIN_WORKER_CRASHED", reason_code="target_monitor_worker_crashed"),
    event(21, "PLUGIN_WORKER_RESTARTED", restarted_generation=restarted),
    event(22, "TARGET_MONITOR_RESTARTED", restarted_generation=restarted),
]
browser = {
    "proof_mode": "real_browser_target_monitor_worker_recovery",
    "session_id": session,
    "subject_ura": subject,
    "same_public_session": True,
    "ordered_worker_events": [event["event_type"] for event in events],
    "daemon_transport_epoch_preserved": True,
    "target_binding_epoch_preserved": True,
    "media_source_epoch_preserved": True,
    "consent_epoch_preserved": True,
    "transport_epoch_before": 101,
    "transport_epoch_after": 101,
    "binding_epoch_before": 3,
    "binding_epoch_after": 3,
    "media_source_epoch_before": 4,
    "media_source_epoch_after": 4,
    "consent_epoch_before": 5,
    "consent_epoch_after": 5,
    "frames_rendered_after_worker_restart": 2,
    "first_frame_rendered_after_worker_restart_at_ms": 40,
    "new_consent_required": False,
    "frontend_status": "remote desktop target monitor recovered",
}
value = {
    "status": "passed",
    "evidence_origin": "contract_self_test",
    "proof_mode": "real_target_monitor_worker_only_recovery",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "self_test": True,
    "session_id": session,
    "selected_resource_ura": subject,
    "process": {
        "before_pid": 1200,
        "after_pid": 1200,
        "before_state_code": "J800",
        "after_state_code": "J800",
    },
    "crash_marker": {
        "schema_version": 1,
        "fault": "crash_target_monitor_generation",
        "session_id": session,
        "pid": 1200,
        "generation": failed,
    },
    "browser_worker_recovery": browser,
    "public_events": events,
    "persisted_events": events,
    "terminal_cleanup": {"completed": True, "session_id": session, "terminal": True},
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

capture_stable_runtime_status() {
  local destination="$1"
  local deadline=$((SECONDS + 30))
  local candidate="$destination.candidate"
  while (( SECONDS < deadline )); do
    if "$EASYNET_BIN" runtime status --json >"$candidate" 2>/dev/null \
      && python3 - "$candidate" <<'PY'
import json
import sys

status = json.load(open(sys.argv[1], encoding="utf-8"))
raise SystemExit(0 if (
    status.get("connection", {}).get("state_code") == "J800"
    and status.get("product_presence", {}).get("session_admitted") is True
    and isinstance(status.get("daemon", {}).get("pid"), int)
    and status["daemon"]["pid"] > 1
) else 1)
PY
    then
      mv "$candidate" "$destination"
      return 0
    fi
    sleep 0.25
  done
  rm -f "$candidate"
  echo "Runtime did not reach stable J800/session_admitted within 30 seconds" >&2
  return 1
}

if [[ "$MODE" == skip ]]; then
  write_report skipped "opt in with --run"
  echo "[host-remoteapp-target-monitor-worker-recovery-e2e] SKIP: $REPORT_MD"
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
  echo "[host-remoteapp-target-monitor-worker-recovery-e2e] SELF-TEST PASS: $REPORT_MD"
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
env "$FAULT_ARM_ENV=$ARM_FILE" "$EASYNET_BIN" runtime start >/dev/null
capture_stable_runtime_status "$RUNTIME_BEFORE_JSON"

arm_command="$(python3 - "$0" "$ARM_FILE" "$MARKER_FILE" <<'PY'
import shlex
import sys
print(" ".join(map(shlex.quote, [sys.argv[1], "--fixture-arm", sys.argv[2], sys.argv[3]])))
PY
)"

env "$FAULT_ARM_ENV=$ARM_FILE" \
  EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON="$BROWSER_EVIDENCE" \
  EASYNET_REMOTEAPP_BROWSER_TARGET_KIND="$TARGET_KIND" \
  EASYNET_REMOTEAPP_BROWSER_TARGET_MONITOR_CRASH_ARM_COMMAND="$arm_command" \
  node "$RUNNER"

"$BROWSER_VERIFIER" --run --evidence-json "$BROWSER_EVIDENCE" \
  --out-dir "$BROWSER_VERIFIED_DIR" >/dev/null
[[ -f "$MARKER_FILE" ]] || { echo "target-monitor crash marker missing" >&2; exit 76; }
capture_stable_runtime_status "$RUNTIME_AFTER_JSON"

SESSION_ID="$(python3 - "$BROWSER_EVIDENCE" <<'PY'
import json, sys
print(json.load(open(sys.argv[1], encoding="utf-8"))["session_id"])
PY
)"
STATE_DIR="$(python3 - <<'PY'
from pathlib import Path
print(Path.home() / ".easynet")
PY
)"
cp "$STATE_DIR/remote-desktop/sessions/$SESSION_ID.json" "$RECOVERY_SNAPSHOT_JSON"
chmod 600 "$RECOVERY_SNAPSHOT_JSON"

python3 - "$EVIDENCE_JSON" "$BROWSER_EVIDENCE" "$MARKER_FILE" \
  "$RECOVERY_SNAPSHOT_JSON" "$RUNTIME_BEFORE_JSON" "$RUNTIME_AFTER_JSON" <<'PY'
import json
import pathlib
import sys

evidence_path, browser_path, marker_path, snapshot_path, before_path, after_path = sys.argv[1:7]
load = lambda path: json.load(open(path, encoding="utf-8"))
browser, marker, snapshot, before, after = map(load, (
    browser_path, marker_path, snapshot_path, before_path, after_path,
))
session_id = browser["session_id"]
subject = browser["selected_resource_ura"]
expected_types = {
    "PLUGIN_WORKER_CRASHED",
    "PLUGIN_WORKER_RESTARTED",
    "TARGET_MONITOR_RESTARTED",
}
public_events = browser["target_monitor_worker_recovery"].get("worker_event_records", [])
persisted_by_type = {
    event["event_type"]: event for event in snapshot.get("events", [])
    if event.get("session_id") == session_id and event.get("event_type") in expected_types
}
persisted_events = [persisted_by_type[event_type] for event_type in (
    "PLUGIN_WORKER_CRASHED", "PLUGIN_WORKER_RESTARTED", "TARGET_MONITOR_RESTARTED",
) if event_type in persisted_by_type]
terminal_candidates = [
    response for response in browser.get("terminal_responses", [])
    if response.get("session_id") == session_id and response.get("completed") is True
]
terminal = terminal_candidates[-1] if terminal_candidates else {}
value = {
    "status": "passed",
    "evidence_origin": "live_runner",
    "proof_mode": "real_target_monitor_worker_only_recovery",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "session_id": session_id,
    "selected_resource_ura": subject,
    "browser_worker_recovery": browser["target_monitor_worker_recovery"],
    "browser_verifier_report": str(pathlib.Path(browser_path).parent / "browser-verifier" / "report.md"),
    "process": {
        "before_pid": before["daemon"]["pid"],
        "after_pid": after["daemon"]["pid"],
        "before_state_code": before["connection"]["state_code"],
        "after_state_code": after["connection"]["state_code"],
    },
    "crash_marker": marker,
    "public_events": public_events,
    "persisted_events": persisted_events,
    "terminal_cleanup": terminal,
}
pathlib.Path(evidence_path).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

validate_evidence
write_report passed "target-monitor generation recovered without replacing daemon, session, consent, binding, transport, or media source"
echo "[host-remoteapp-target-monitor-worker-recovery-e2e] PASS: $REPORT_MD"
