#!/usr/bin/env bash
# Aggregate two independently verified Browser RemoteApp target lifecycles.

set -euo pipefail

MODE=skip
WINDOW_REPORT="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_WINDOW_REPORT_JSON:-}"
APPLICATION_REPORT="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_APPLICATION_REPORT_JSON:-}"
EXPECTED_INPUT_MODE="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EXPECTED_INPUT_MODE:-interactive}"
OUT_DIR="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_MATRIX_OUT_DIR:-$(pwd)/target/e2e/frontend-remoteapp-browser-lifecycle-matrix/$(date -u +%Y%m%d-%H%M%S)-$$}"

usage() {
  cat <<'USAGE'
Usage:
  frontend-remoteapp-browser-lifecycle-matrix-e2e.sh --run \
    --window-report PATH --application-report PATH \
    [--expected-input-mode interactive|view_only] [--out-dir DIR]
  frontend-remoteapp-browser-lifecycle-matrix-e2e.sh --self-test [--out-dir DIR]

This is a matrix aggregator, not a Browser runner. Each input must be a passed
leaf report from frontend-remoteapp-browser-lifecycle-e2e.sh with its own live
evidence JSON. Interactive matrices require host-applied input and target-blur
focus recovery. View-only matrices require an explicit policy block and do not
claim target-local input support.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
    --window-report) WINDOW_REPORT="${2:?missing window report}"; shift 2 ;;
    --application-report) APPLICATION_REPORT="${2:?missing application report}"; shift 2 ;;
    --expected-input-mode) EXPECTED_INPUT_MODE="${2:?missing input mode}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing output directory}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

case "$EXPECTED_INPUT_MODE" in
  interactive|view_only) ;;
  *) echo "invalid expected input mode: $EXPECTED_INPUT_MODE" >&2; exit 64 ;;
esac

mkdir -p "$OUT_DIR"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
EVIDENCE_JSON="$OUT_DIR/evidence.json"

if [[ "$MODE" == skip ]]; then
  python3 - "$REPORT_JSON" "$REPORT_MD" <<'PY'
import json, pathlib, sys
report = {
    "script": "tools/scripts/frontend-remoteapp-browser-lifecycle-matrix-e2e.sh",
    "status": "skipped",
    "reason": "pass --run with window and application leaf reports",
    "product_complete_claim": False,
}
pathlib.Path(sys.argv[1]).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
pathlib.Path(sys.argv[2]).write_text("# Frontend RemoteApp Browser Target Matrix\n\n- Status: `skipped`\n", encoding="utf-8")
PY
  echo "[frontend-remoteapp-browser-lifecycle-matrix-e2e] skipped"
  exit 0
fi

if [[ "$MODE" == self-test ]]; then
  WINDOW_REPORT="$OUT_DIR/window-leaf-report.json"
  APPLICATION_REPORT="$OUT_DIR/application-leaf-report.json"
  python3 - "$OUT_DIR" "$WINDOW_REPORT" "$APPLICATION_REPORT" <<'PY'
import json, pathlib, sys
out = pathlib.Path(sys.argv[1])
for index, (kind, report_path) in enumerate((
    ("window", pathlib.Path(sys.argv[2])),
    ("application", pathlib.Path(sys.argv[3])),
), start=1):
    evidence_path = out / f"{kind}-leaf-evidence.json"
    subject = f"easynet:///r/localhost/resource/device.test/streams/{kind}.test"
    evidence = {
        "status": "passed",
        "evidence_origin": "contract_self_test",
        "proof_mode": "real_browser_tauri_lifecycle",
        "selected_target_kind": kind,
        "selected_resource_ura": subject,
        "session_id": f"rdp-{kind}-self-test",
        "product_complete_claim": False,
    }
    evidence_path.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    report = {
        "script": "tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh",
        "status": "passed",
        "evidence_origin": "contract_self_test",
        "target_kind": kind,
        "session_id": evidence["session_id"],
        "selected_resource_ura": subject,
        "input_result": "input_applied",
        "input_interaction_sequence_verified": True,
        "focus_recovery_verified": True,
        "interactive_target_kinds": [kind],
        "evidence_json": str(evidence_path),
        "product_complete_claim": False,
    }
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
PY
fi

[[ -n "$WINDOW_REPORT" && -n "$APPLICATION_REPORT" ]] || {
  echo "[FAIL] both --window-report and --application-report are required" >&2
  exit 64
}

python3 - "$MODE" "$EXPECTED_INPUT_MODE" "$WINDOW_REPORT" "$APPLICATION_REPORT" "$EVIDENCE_JSON" "$REPORT_JSON" "$REPORT_MD" <<'PY'
import hashlib
import json
import pathlib
import sys

mode, expected_input_mode, window_path_text, application_path_text, evidence_path_text, report_path_text, md_path_text = sys.argv[1:]
expected_origin = "contract_self_test" if mode == "self-test" else "live_runner"
errors = []
leaves = []

def require(condition, message):
    if not condition:
        errors.append(message)

def read_object(path, label):
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except Exception as error:
        errors.append(f"{label}: cannot read JSON: {error}")
        return {}
    if not isinstance(value, dict):
        errors.append(f"{label}: JSON must be an object")
        return {}
    return value

for kind, path_text in (("window", window_path_text), ("application", application_path_text)):
    report_path = pathlib.Path(path_text).resolve()
    report = read_object(report_path, f"{kind} report")
    require(report.get("script") == "tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh",
            f"{kind} report must come from the Browser lifecycle leaf verifier")
    require(report.get("status") == "passed", f"{kind} report status must be passed")
    require(report.get("evidence_origin") == expected_origin,
            f"{kind} report evidence_origin must be {expected_origin}")
    require(report.get("target_kind") == kind, f"{kind} report target_kind must be {kind}")
    if expected_input_mode == "interactive":
        require(report.get("input_result") == "input_applied",
                f"{kind} report must prove input_result=input_applied")
        require(report.get("input_interaction_sequence_verified") is True,
                f"{kind} report must verify pointer/key down/up")
        require(report.get("focus_recovery_verified") is True,
                f"{kind} report must verify target-blur focus recovery")
        require(report.get("interactive_target_kinds") == [kind],
                f"{kind} report interactive_target_kinds must contain only {kind}")
    else:
        require(report.get("input_result") == "policy_blocked",
                f"{kind} report must prove input_result=policy_blocked")
        require(report.get("input_interaction_sequence_verified") is False,
                f"{kind} view-only report must not claim pointer/key application")
        require(report.get("focus_recovery_verified") is False,
                f"{kind} view-only report must not claim target-blur focus recovery")
        require(report.get("interactive_target_kinds") == [],
                f"{kind} view-only report must not claim interactive target kinds")
    require(report.get("product_complete_claim") is not True,
            f"{kind} leaf must not claim product completion")
    evidence_path_value = report.get("evidence_json")
    require(isinstance(evidence_path_value, str) and bool(evidence_path_value),
            f"{kind} report evidence_json must be set")
    if not isinstance(evidence_path_value, str) or not evidence_path_value:
        continue
    evidence_path = pathlib.Path(evidence_path_value).resolve()
    evidence = read_object(evidence_path, f"{kind} evidence")
    require(evidence.get("status") == "passed", f"{kind} evidence status must be passed")
    require(evidence.get("evidence_origin") == expected_origin,
            f"{kind} evidence_origin must be {expected_origin}")
    require(evidence.get("proof_mode") == "real_browser_tauri_lifecycle",
            f"{kind} evidence proof_mode must be real_browser_tauri_lifecycle")
    require(evidence.get("selected_target_kind") == kind,
            f"{kind} evidence selected_target_kind must be {kind}")
    require(evidence.get("session_id") == report.get("session_id"),
            f"{kind} report/evidence session_id must match")
    require(evidence.get("selected_resource_ura") == report.get("selected_resource_ura"),
            f"{kind} report/evidence Resource URA must match")
    subject = evidence.get("selected_resource_ura")
    require(isinstance(subject, str) and subject.startswith("easynet:///"),
            f"{kind} Resource URA must be canonical")
    leaves.append({
        "target_kind": kind,
        "report_json": str(report_path),
        "report_sha256": hashlib.sha256(report_path.read_bytes()).hexdigest(),
        "evidence_json": str(evidence_path),
        "evidence_sha256": hashlib.sha256(evidence_path.read_bytes()).hexdigest(),
        "session_id": evidence.get("session_id"),
        "selected_resource_ura": subject,
        "input_interaction_sequence_verified": report.get("input_interaction_sequence_verified") is True,
        "focus_recovery_verified": report.get("focus_recovery_verified") is True,
    })

require(len(leaves) == 2, "matrix must contain two valid target leaves")
if len(leaves) == 2:
    require(leaves[0]["session_id"] != leaves[1]["session_id"],
            "window and application leaves must be independent sessions")
    require(leaves[0]["selected_resource_ura"] != leaves[1]["selected_resource_ura"],
            "window and application leaves must bind distinct Resource URAs")

status = "failed" if errors else "passed"
evidence = {
    "status": status,
    "evidence_origin": expected_origin,
    "proof_mode": "real_browser_lifecycle_target_matrix",
    "component_mock": False,
    "real_backend_runtime": True,
    "expected_input_mode": expected_input_mode,
    "interactive_target_kinds": ["application", "window"] if expected_input_mode == "interactive" else [],
    "view_only_target_kinds": ["application", "window"] if expected_input_mode == "view_only" else [],
    "targets": leaves,
    "product_complete_claim": False,
}
evidence_path = pathlib.Path(evidence_path_text)
evidence_path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
report = {
    "script": "tools/scripts/frontend-remoteapp-browser-lifecycle-matrix-e2e.sh",
    "status": status,
    "evidence_origin": expected_origin,
    "errors": errors,
    "target_kind": "both",
    "expected_input_mode": expected_input_mode,
    "input_result": ("input_applied" if expected_input_mode == "interactive" else "policy_blocked") if not errors else "failed",
    "input_interaction_sequence_verified": not errors and expected_input_mode == "interactive",
    "focus_recovery_verified": not errors and expected_input_mode == "interactive",
    "focus_recovery_target_kinds": ["application", "window"] if not errors and expected_input_mode == "interactive" else [],
    "interactive_target_kinds": ["application", "window"] if not errors and expected_input_mode == "interactive" else [],
    "view_only_target_kinds": ["application", "window"] if not errors and expected_input_mode == "view_only" else [],
    "leaf_reports": leaves,
    "evidence_json": str(evidence_path.resolve()),
    "product_complete_claim": False,
}
pathlib.Path(report_path_text).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
lines = [
    "# Frontend RemoteApp Browser Target Matrix",
    "",
    f"- Status: `{status}`",
    f"- Evidence origin: `{expected_origin}`",
    f"- Evidence: `{evidence_path.resolve()}`",
]
if errors:
    lines.extend(["", "## Errors", "", *[f"- {error}" for error in errors]])
pathlib.Path(md_path_text).write_text("\n".join(lines) + "\n", encoding="utf-8")
if errors:
    for error in errors:
        print(f"[FAIL] {error}", file=sys.stderr)
    raise SystemExit(1)
PY

echo "[frontend-remoteapp-browser-lifecycle-matrix-e2e] PASS: $REPORT_MD"
