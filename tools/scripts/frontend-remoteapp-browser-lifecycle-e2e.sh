#!/usr/bin/env bash
# Frontend RemoteApp Browser/Tauri lifecycle E2E evidence verifier.
#
# Boundary:
# - This harness verifies evidence produced by a real browser/Tauri runner for
#   the frontend RemoteApp lifecycle. It does not replace daemon/host E2E
#   harnesses and does not simulate browser UI actions.
# - A live pass requires either --evidence-json from an external runner or
#   --runner-cmd that writes the evidence JSON path provided through
#   EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON.
# - Self-test validates the evidence contract only; it is not product evidence.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

MODE=skip
SELF_TEST=0
OUT_DIR="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_OUT_DIR:-$REPO_ROOT/target/e2e/frontend-remoteapp-browser-lifecycle/$(date -u +%Y%m%d-%H%M%S)-$$}"
FRONTEND_URL="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL:-}"
SURFACE="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_SURFACE:-browser}"
RUNNER_CMD="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_RUNNER_CMD:-}"
EVIDENCE_INPUT="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON:-}"

usage() {
  cat <<'USAGE'
Usage:
  frontend-remoteapp-browser-lifecycle-e2e.sh --run --evidence-json PATH
  frontend-remoteapp-browser-lifecycle-e2e.sh --run --runner-cmd CMD --frontend-url URL
  frontend-remoteapp-browser-lifecycle-e2e.sh --self-test

Options:
  --run                 Verify real Browser/Tauri lifecycle evidence.
  --self-test           Validate the harness against synthetic positive evidence.
  --frontend-url URL    Browser/Tauri app URL used by the external runner.
  --surface KIND        browser or tauri. Default: browser.
  --runner-cmd CMD      Command that drives the real UI and writes evidence to
                        EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON.
  --evidence-json PATH  Existing evidence JSON emitted by a real UI runner.
  --out-dir DIR         Report directory.
  -h, --help            Show this help.

Environment:
  EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_E2E=1
                        Equivalent to --run.

Evidence contract:
  The evidence JSON must prove a real Browser/Tauri flow, not component mocks:
  app_loaded -> authenticated_session -> target_picker_opened ->
  permission_status_checked -> consent_granted -> session_created ->
  webrtc_attached -> watch_events_streaming -> media_presented ->
  input_control_attempted_or_policy_blocked -> session_ended ->
  terminal_receipt_visible.

Non-claims:
  A skipped report or self-test does not prove frontend product readiness.
  This harness verifies one Browser/Tauri lifecycle artifact; cross-device,
  OS input injection, codec soak, and network fallback still require their own
  evidence.
USAGE
}

if [[ "${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_E2E:-0}" == "1" ]]; then
  MODE=run
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) SELF_TEST=1; MODE=self-test; shift ;;
    --frontend-url) FRONTEND_URL="${2:?missing value for --frontend-url}"; shift 2 ;;
    --surface)
      case "${2:?missing value for --surface}" in
        browser|tauri) SURFACE="$2" ;;
        *) echo "invalid surface: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --runner-cmd) RUNNER_CMD="${2:?missing value for --runner-cmd}"; shift 2 ;;
    --evidence-json) EVIDENCE_INPUT="${2:?missing value for --evidence-json}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

mkdir -p "$OUT_DIR"
EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
RUNNER_STDOUT="$OUT_DIR/runner.stdout.txt"
RUNNER_STDERR="$OUT_DIR/runner.stderr.txt"

write_report() {
  local status="$1"
  local reason="$2"
  python3 - "$REPORT_JSON" "$REPORT_MD" "$status" "$reason" "$SURFACE" "$FRONTEND_URL" "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

report_path, md_path, status, reason, surface, frontend_url, evidence_path = sys.argv[1:8]
report = {
    "script": "tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh",
    "status": status,
    "reason": reason,
    "surface": surface,
    "frontend_url": frontend_url,
    "evidence_json": evidence_path,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# Frontend RemoteApp Browser/Tauri Lifecycle E2E\n\n"
    f"- Status: `{status}`\n"
    f"- Surface: `{surface}`\n"
    f"- Frontend URL: `{frontend_url}`\n"
    f"- Reason: `{reason}`\n"
    f"- Evidence: `{evidence_path}`\n",
    encoding="utf-8",
)
PY
}

validate_evidence() {
  python3 - "$EVIDENCE_JSON" "$REPORT_JSON" "$REPORT_MD" <<'PY'
import json
import pathlib
import sys

evidence_path, report_path, md_path = sys.argv[1:4]
with open(evidence_path, encoding="utf-8") as f:
    evidence = json.load(f)

errors = []

def require(condition, message):
    if not condition:
        errors.append(message)

def get(path, default=None):
    value = evidence
    for part in path.split("."):
        if not isinstance(value, dict) or part not in value:
            return default
        value = value[part]
    return value

required_steps = [
    "app_loaded",
    "authenticated_session",
    "target_picker_opened",
    "permission_status_checked",
    "consent_granted",
    "session_created",
    "webrtc_attached",
    "watch_events_streaming",
    "media_presented",
    "input_control_attempted_or_policy_blocked",
    "session_ended",
    "terminal_receipt_visible",
]
ability_steps = {
    "permission_status_checked": "remote_desktop.permission_status",
    "consent_granted": "remote_desktop.grant_consent",
    "session_created": "remote_desktop.create_session",
    "webrtc_attached": "remote_desktop.attach",
    "watch_events_streaming": "remote_desktop.watch_events",
    "session_ended": "remote_desktop.end_session",
}

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("proof_mode") == "real_browser_tauri_lifecycle",
        "proof_mode must be real_browser_tauri_lifecycle")
require(evidence.get("runner_kind") in {"browser", "tauri"},
        "runner_kind must be browser or tauri")
require(evidence.get("component_mock") is False,
        "component_mock must be false")
require(evidence.get("real_backend_runtime") is True,
        "real_backend_runtime must be true")
require(evidence.get("product_complete_claim") is False,
        "product_complete_claim must remain false")
require(isinstance(evidence.get("frontend_url"), str)
        and evidence["frontend_url"].startswith(("http://", "https://", "tauri://")),
        "frontend_url must identify the real frontend surface")

device_ura = evidence.get("device_ura")
subject_ura = evidence.get("selected_resource_ura")
session_id = evidence.get("session_id")
require(isinstance(device_ura, str) and device_ura.startswith("easynet:///"),
        "device_ura must be a canonical EasyNet URA")
require(isinstance(subject_ura, str) and subject_ura.startswith("easynet:///"),
        "selected_resource_ura must be a canonical EasyNet Resource URA")
require(isinstance(session_id, str) and session_id,
        "session_id must be recorded")

steps = evidence.get("steps")
require(isinstance(steps, list) and steps, "steps must be a non-empty list")
step_names = []
step_by_name = {}
if isinstance(steps, list):
    for step in steps:
        if not isinstance(step, dict):
            errors.append("each step must be an object")
            continue
        name = step.get("name")
        if not isinstance(name, str):
            errors.append("each step must have a name")
            continue
        step_names.append(name)
        step_by_name[name] = step
        require(step.get("status") == "passed", f"{name}: status must be passed")

cursor = -1
for required in required_steps:
    try:
        index = step_names.index(required)
    except ValueError:
        errors.append(f"missing lifecycle step: {required}")
        continue
    require(index > cursor, f"lifecycle step order is wrong at {required}")
    cursor = index

for step_name, ability in ability_steps.items():
    step = step_by_name.get(step_name)
    if not isinstance(step, dict):
        continue
    require(step.get("ability") == ability, f"{step_name}: ability must be {ability}")
    if step_name != "permission_status_checked":
        require(step.get("subject_ura") == subject_ura,
                f"{step_name}: subject_ura must equal selected Resource URA")
    else:
        require(step.get("subject_ura") in {None, ""},
                "permission_status_checked must be host-local and not target-scoped")

created = step_by_name.get("session_created", {})
ended = step_by_name.get("session_ended", {})
terminal = step_by_name.get("terminal_receipt_visible", {})
watch = step_by_name.get("watch_events_streaming", {})
media = step_by_name.get("media_presented", {})
input_step = step_by_name.get("input_control_attempted_or_policy_blocked", {})
require(created.get("session_id") == session_id,
        "session_created must bind the top-level session_id")
require(watch.get("session_id") == session_id,
        "watch_events_streaming must bind the created session_id")
require(ended.get("session_id") == session_id,
        "session_ended must bind the created session_id")
require(media.get("frame_presented") is True,
        "media_presented must prove at least one rendered media frame")
require(input_step.get("result") in {"input_applied", "policy_blocked"},
        "input control must either apply input or prove policy_blocked")
require(terminal.get("reason_code") in {"user_cancelled", "caller_ended", "resume_e2e_cleanup"},
        "terminal_receipt_visible must expose a known end reason")
require(terminal.get("terminal") is True,
        "terminal_receipt_visible must expose terminal=true")
require(terminal.get("session_id") == session_id,
        "terminal receipt must bind the created session id")

report = {
    "script": "tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh",
    "status": "failed" if errors else "passed",
    "errors": errors,
    "surface": evidence.get("runner_kind"),
    "frontend_url": evidence.get("frontend_url"),
    "session_id": session_id,
    "selected_resource_ura": subject_ura,
    "evidence_json": evidence_path,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# Frontend RemoteApp Browser/Tauri Lifecycle E2E\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Surface: `{report['surface']}`\n")
    f.write(f"- Frontend URL: `{report['frontend_url']}`\n")
    f.write(f"- Session id: `{report['session_id']}`\n")
    f.write(f"- Selected Resource URA: `{report['selected_resource_ura']}`\n")
    f.write(f"- Evidence: `{evidence_path}`\n")
    if errors:
        f.write("\n## Errors\n")
        for error in errors:
            f.write(f"- {error}\n")
if errors:
    for error in errors:
        print(error, file=sys.stderr)
    raise SystemExit(1)
PY
}

if [[ "$SELF_TEST" -eq 1 ]]; then
  python3 - "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

subject = "easynet:///r/localhost/resource/device.mac-1/streams/window.browser-lifecycle"
session_id = "rd-browser-lifecycle-self-test"
steps = [
    {"name": "app_loaded", "status": "passed"},
    {"name": "authenticated_session", "status": "passed"},
    {"name": "target_picker_opened", "status": "passed"},
    {"name": "permission_status_checked", "status": "passed", "ability": "remote_desktop.permission_status", "subject_ura": None},
    {"name": "consent_granted", "status": "passed", "ability": "remote_desktop.grant_consent", "subject_ura": subject},
    {"name": "session_created", "status": "passed", "ability": "remote_desktop.create_session", "subject_ura": subject, "session_id": session_id},
    {"name": "webrtc_attached", "status": "passed", "ability": "remote_desktop.attach", "subject_ura": subject, "session_id": session_id},
    {"name": "watch_events_streaming", "status": "passed", "ability": "remote_desktop.watch_events", "subject_ura": subject, "session_id": session_id},
    {"name": "media_presented", "status": "passed", "frame_presented": True},
    {"name": "input_control_attempted_or_policy_blocked", "status": "passed", "result": "policy_blocked"},
    {"name": "session_ended", "status": "passed", "ability": "remote_desktop.end_session", "subject_ura": subject, "session_id": session_id},
    {"name": "terminal_receipt_visible", "status": "passed", "terminal": True, "reason_code": "user_cancelled", "session_id": session_id},
]
evidence = {
    "status": "passed",
    "proof_mode": "real_browser_tauri_lifecycle",
    "runner_kind": "browser",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "frontend_url": "http://127.0.0.1:3000/devices/mac-1",
    "device_ura": "easynet:///r/localhost/device/mac-1",
    "selected_resource_ura": subject,
    "session_id": session_id,
    "steps": steps,
}
pathlib.Path(sys.argv[1]).write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  validate_evidence
  echo "frontend-remoteapp-browser-lifecycle-e2e self-test ok"
  exit 0
fi

if [[ "$MODE" != "run" ]]; then
  write_report "skipped" "set EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_E2E=1 or pass --run"
  echo "[frontend-remoteapp-browser-lifecycle-e2e] skipped: $REPORT_MD"
  exit 0
fi

if [[ -n "$EVIDENCE_INPUT" ]]; then
  [[ -f "$EVIDENCE_INPUT" ]] || {
    write_report "failed" "evidence json does not exist: $EVIDENCE_INPUT"
    echo "[frontend-remoteapp-browser-lifecycle-e2e] missing evidence json: $EVIDENCE_INPUT" >&2
    exit 1
  }
  cp "$EVIDENCE_INPUT" "$EVIDENCE_JSON"
elif [[ -n "$RUNNER_CMD" ]]; then
  [[ -n "$FRONTEND_URL" ]] || {
    write_report "failed" "--frontend-url is required when --runner-cmd is used"
    echo "[frontend-remoteapp-browser-lifecycle-e2e] --frontend-url is required" >&2
    exit 1
  }
  export EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON="$EVIDENCE_JSON"
  export EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL="$FRONTEND_URL"
  export EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_SURFACE="$SURFACE"
  if ! bash -lc "$RUNNER_CMD" >"$RUNNER_STDOUT" 2>"$RUNNER_STDERR"; then
    write_report "failed" "runner command failed"
    echo "[frontend-remoteapp-browser-lifecycle-e2e] runner command failed" >&2
    cat "$RUNNER_STDERR" >&2 || true
    exit 1
  fi
  [[ -f "$EVIDENCE_JSON" ]] || {
    write_report "failed" "runner did not write evidence json"
    echo "[frontend-remoteapp-browser-lifecycle-e2e] runner did not write $EVIDENCE_JSON" >&2
    exit 1
  }
else
  write_report "failed" "--run requires --evidence-json or --runner-cmd"
  echo "[frontend-remoteapp-browser-lifecycle-e2e] --run requires --evidence-json or --runner-cmd" >&2
  exit 1
fi

validate_evidence
echo "[frontend-remoteapp-browser-lifecycle-e2e] PASS: $REPORT_MD"
