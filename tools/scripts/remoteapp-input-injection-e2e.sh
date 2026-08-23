#!/usr/bin/env bash
# RemoteApp input injection E2E evidence verifier.
#
# Boundary:
# - This harness verifies evidence produced by real host runners for pointer
#   and keyboard input injection. It does not simulate OS input.
# - High-frequency pointer/key frames remain on the negotiated session data
#   channel; this harness only verifies session setup, permission, consent,
#   applied events, latency, and terminal evidence.
# - A live pass requires either --evidence-json from an external runner or
#   --runner-cmd that writes the evidence JSON path provided through
#   EASYNET_REMOTEAPP_INPUT_INJECTION_EVIDENCE_JSON.
# - Self-test validates the evidence contract only; it is not product evidence.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

MODE=skip
SELF_TEST=0
OUT_DIR="${EASYNET_REMOTEAPP_INPUT_INJECTION_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-input-injection/$(date -u +%Y%m%d-%H%M%S)-$$}"
RUNNER_CMD="${EASYNET_REMOTEAPP_INPUT_INJECTION_RUNNER_CMD:-}"
EVIDENCE_INPUT="${EASYNET_REMOTEAPP_INPUT_INJECTION_EVIDENCE_JSON:-}"

usage() {
  cat <<'USAGE'
Usage:
  remoteapp-input-injection-e2e.sh --run --evidence-json PATH
  remoteapp-input-injection-e2e.sh --run --runner-cmd CMD
  remoteapp-input-injection-e2e.sh --self-test

Options:
  --run                 Verify real RemoteApp input injection evidence.
  --self-test           Validate the harness against synthetic positive evidence.
  --runner-cmd CMD      Command that drives real host input injection and writes
                        evidence to EASYNET_REMOTEAPP_INPUT_INJECTION_EVIDENCE_JSON.
  --evidence-json PATH  Existing evidence JSON emitted by real host runners.
  --out-dir DIR         Report directory.
  -h, --help            Show this help.

Environment:
  EASYNET_REMOTEAPP_INPUT_INJECTION_E2E=1
                        Equivalent to --run.

Evidence contract:
  The evidence JSON must prove real pointer/keyboard OS input injection, not
  policy-only readiness. macOS must pass pointer and keyboard injection with
  Accessibility/input permission, input-control consent, display_global input
  scope, focus validation, coordinate mapping, target geometry revision,
  strictly ordered INPUT_FRAME_APPLIED events, bounded receive/apply latency,
  stale client-sequence rejection, and a visible terminal receipt. Windows/Linux
  must pass or report explicit product unsupported state.

Non-claims:
  A skipped report or self-test does not prove input product readiness.
  This harness verifies one input artifact; capture, network fallback, codec
  soak, frontend Browser/Tauri lifecycle, and cross-device product behavior
  still require their own evidence.
USAGE
}

if [[ "${EASYNET_REMOTEAPP_INPUT_INJECTION_E2E:-0}" == "1" ]]; then
  MODE=run
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) SELF_TEST=1; MODE=self-test; shift ;;
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
  python3 - "$REPORT_JSON" "$REPORT_MD" "$status" "$reason" "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

report_path, md_path, status, reason, evidence_path = sys.argv[1:6]
coverage = {"macos": False, "windows": False, "linux": False}
report = {
    "script": "tools/scripts/remoteapp-input-injection-e2e.sh",
    "status": status,
    "reason": reason,
    "evidence_json": evidence_path,
    "coverage": coverage,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# RemoteApp Input Injection E2E\n\n"
    f"- Status: `{status}`\n"
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

def is_ura(value):
    return isinstance(value, str) and value.startswith("easynet:///")

def normalize(value):
    return value.lower() if isinstance(value, str) else value

required_platforms = {"macos", "windows", "linux"}
required_inputs = {"pointer", "keyboard"}
terminal_reasons = {"caller_ended", "user_cancelled", "input_injection_e2e_cleanup"}

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("proof_mode") == "real_input_injection_matrix",
        "proof_mode must be real_input_injection_matrix")
require(evidence.get("component_mock") is False,
        "component_mock must be false")
require(evidence.get("real_backend_runtime") is True,
        "real_backend_runtime must be true")
require(evidence.get("product_complete_claim") is False,
        "product_complete_claim must remain false")

latency_threshold = evidence.get("latency_threshold_ms", 100)
try:
    latency_threshold = float(latency_threshold)
except (TypeError, ValueError):
    latency_threshold = 100.0
require(0 < latency_threshold <= 250,
        "latency_threshold_ms must be positive and no higher than 250")

platforms = evidence.get("platforms")
require(isinstance(platforms, list) and platforms, "platforms must be a non-empty list")
platform_by_name = {}
if isinstance(platforms, list):
    for platform in platforms:
        if not isinstance(platform, dict):
            errors.append("each platform entry must be an object")
            continue
        name = normalize(platform.get("platform"))
        if name in platform_by_name:
            errors.append(f"duplicate platform entry: {name}")
        platform_by_name[name] = platform

missing_platforms = sorted(required_platforms - set(platform_by_name))
require(not missing_platforms, "missing platform evidence: " + ", ".join(missing_platforms))

platform_reports = []
for platform_name in sorted(required_platforms):
    platform = platform_by_name.get(platform_name)
    if not isinstance(platform, dict):
        continue
    status = platform.get("status")
    prefix = platform_name
    if status == "passed":
        subject_ura = platform.get("selected_resource_ura")
        session_id = platform.get("session_id")
        require(is_ura(subject_ura), f"{prefix}: selected_resource_ura must be canonical")
        require(isinstance(session_id, str) and session_id, f"{prefix}: session_id must be recorded")
        require(platform.get("permission", {}).get("input_injection_granted") is True
                or platform.get("permission", {}).get("accessibility_granted") is True,
                f"{prefix}: OS input permission must be granted")
        require(platform.get("consent_scope") == "input_control",
                f"{prefix}: consent_scope must be input_control")
        require(platform.get("input_scope") == "display_global",
                f"{prefix}: input_scope must be display_global")
        require(platform.get("focus_validated") is True,
                f"{prefix}: focus_validated must be true")
        require(platform.get("coordinate_mapping_validated") is True,
                f"{prefix}: coordinate_mapping_validated must be true")
        require(isinstance(platform.get("target_geometry_revision"), int)
                and platform.get("target_geometry_revision") > 0,
                f"{prefix}: target_geometry_revision must be positive")
        require(platform.get("source_only_proof") is False,
                f"{prefix}: source_only_proof must be false")
        require(platform.get("policy_only") is False,
                f"{prefix}: policy_only must be false")

        abilities = platform.get("abilities")
        require(isinstance(abilities, list) and abilities,
                f"{prefix}: abilities must be non-empty")
        ability_by_name = {}
        if isinstance(abilities, list):
            for ability in abilities:
                if isinstance(ability, dict) and isinstance(ability.get("name"), str):
                    ability_by_name[ability["name"]] = ability
        for ability_name in (
            "remote_desktop.create_session",
            "remote_desktop.attach",
            "remote_desktop.watch_events",
            "remote_desktop.end_session",
        ):
            ability = ability_by_name.get(ability_name)
            require(isinstance(ability, dict), f"{prefix}: missing ability {ability_name}")
            if isinstance(ability, dict):
                require(ability.get("subject_ura") == subject_ura,
                        f"{prefix}: {ability_name} must bind selected Resource URA")
                if ability_name != "remote_desktop.create_session":
                    require(ability.get("session_id") == session_id,
                            f"{prefix}: {ability_name} must bind session_id")

        input_results = platform.get("input_results")
        require(isinstance(input_results, list) and input_results,
                f"{prefix}: input_results must be non-empty")
        applied_sequences = []
        last_sequence = 0
        result_by_kind = {}
        if isinstance(input_results, list):
            for result in input_results:
                if isinstance(result, dict) and isinstance(result.get("kind"), str):
                    sequence = result.get("client_sequence")
                    if isinstance(sequence, int):
                        require(sequence > last_sequence,
                                f"{prefix}: input_results client_sequence must be strictly increasing")
                        last_sequence = sequence
                        applied_sequences.append(sequence)
                    result_by_kind[result["kind"]] = result
        missing_inputs = sorted(required_inputs - set(result_by_kind))
        require(not missing_inputs, f"{prefix}: missing input results: " + ", ".join(missing_inputs))
        latencies = []
        for kind in sorted(required_inputs):
            result = result_by_kind.get(kind)
            if not isinstance(result, dict):
                continue
            result_prefix = f"{prefix}/{kind}"
            require(result.get("result") == "input_applied",
                    f"{result_prefix}: result must be input_applied")
            require(result.get("event_type") == "INPUT_FRAME_APPLIED",
                    f"{result_prefix}: event_type must be INPUT_FRAME_APPLIED")
            require(isinstance(result.get("client_sequence"), int)
                    and result.get("client_sequence") > 0,
                    f"{result_prefix}: client_sequence must be positive")
            require(isinstance(result.get("client_sent_at_ms"), int)
                    and result.get("client_sent_at_ms") > 0,
                    f"{result_prefix}: client_sent_at_ms must be positive")
            require(isinstance(result.get("host_received_at_ms"), int)
                    and result.get("host_received_at_ms") >= result.get("client_sent_at_ms", 0),
                    f"{result_prefix}: host_received_at_ms must be >= client_sent_at_ms")
            require(isinstance(result.get("host_applied_at_ms"), int)
                    and result.get("host_applied_at_ms") >= result.get("host_received_at_ms", 0),
                    f"{result_prefix}: host_applied_at_ms must be >= host_received_at_ms")
            latency = result.get("latency_ms")
            try:
                latency_value = float(latency)
            except (TypeError, ValueError):
                latency_value = -1.0
            latencies.append(latency_value)
            require(0 <= latency_value <= latency_threshold,
                    f"{result_prefix}: latency_ms must be within threshold")
            require(result.get("observed_effect") in {
                "pointer_position_changed",
                "key_echo_observed",
                "key_event_observed",
            }, f"{result_prefix}: observed_effect must prove OS input effect")
            if kind == "pointer":
                require(result.get("coordinate_mapping") == "target_geometry_revision_matched",
                        f"{result_prefix}: coordinate mapping must bind target_geometry_revision")
                require(result.get("target_geometry_revision") == platform.get("target_geometry_revision"),
                        f"{result_prefix}: target_geometry_revision must match platform scenario")
            if kind == "keyboard":
                require(isinstance(result.get("key_code"), str) and result.get("key_code"),
                        f"{result_prefix}: key_code must be recorded")

        rejected_results = platform.get("rejected_input_results")
        require(isinstance(rejected_results, list) and rejected_results,
                f"{prefix}: rejected_input_results must include stale sequence rejection evidence")
        stale_rejections = []
        if isinstance(rejected_results, list):
            for rejection in rejected_results:
                if not isinstance(rejection, dict):
                    continue
                if (rejection.get("event_type") == "INPUT_FRAME_REJECTED"
                        and rejection.get("reason") == "stale_client_sequence"):
                    stale_rejections.append(rejection)
        require(stale_rejections,
                f"{prefix}: stale_client_sequence rejection must be observed")
        max_applied_sequence = max(applied_sequences) if applied_sequences else 0
        for index, rejection in enumerate(stale_rejections):
            rejection_prefix = f"{prefix}/stale_rejection[{index}]"
            require(rejection.get("subject_ura") == subject_ura,
                    f"{rejection_prefix}: subject_ura must bind selected Resource URA")
            require(rejection.get("session_id") == session_id,
                    f"{rejection_prefix}: session_id must bind session_id")
            require(isinstance(rejection.get("client_sequence"), int)
                    and 0 < rejection.get("client_sequence") <= max_applied_sequence,
                    f"{rejection_prefix}: client_sequence must be stale against applied input")
            require("host_applied_at_ms" not in rejection or rejection.get("host_applied_at_ms") in {None, ""},
                    f"{rejection_prefix}: stale rejected input must not be host-applied")

        latency_summary = platform.get("latency_summary")
        require(isinstance(latency_summary, dict), f"{prefix}: latency_summary must be present")
        if not isinstance(latency_summary, dict):
            latency_summary = {}
        require(float(latency_summary.get("p95_ms", latency_threshold + 1)) <= latency_threshold,
                f"{prefix}: latency_summary.p95_ms must be within threshold")
        require(float(latency_summary.get("max_ms", latency_threshold + 1)) <= latency_threshold,
                f"{prefix}: latency_summary.max_ms must be within threshold")

        terminal = platform.get("terminal_receipt")
        require(isinstance(terminal, dict), f"{prefix}: terminal_receipt must be visible")
        if not isinstance(terminal, dict):
            terminal = {}
        require(terminal.get("terminal") is True,
                f"{prefix}: terminal_receipt.terminal must be true")
        require(terminal.get("session_id") == session_id,
                f"{prefix}: terminal_receipt must bind session_id")
        require(terminal.get("reason_code") in terminal_reasons,
                f"{prefix}: terminal_receipt.reason_code must be a known cleanup/end reason")

        platform_reports.append({
            "platform": platform_name,
            "status": "passed",
            "input_results": sorted(result_by_kind),
            "stale_client_sequence_rejected": bool(stale_rejections),
            "max_latency_ms": max(latencies) if latencies else None,
        })
    elif status == "unsupported":
        require(platform_name in {"windows", "linux"},
                f"{prefix}: unsupported input state is allowed only on Windows/Linux")
        require(platform.get("unsupported_state") == "explicit_product_unsupported",
                f"{prefix}: unsupported_state must be explicit_product_unsupported")
        require(platform.get("show_unsupported") is True,
                f"{prefix}: show_unsupported must be true")
        require(platform.get("input_results") is None or platform.get("input_results") == [],
                f"{prefix}: unsupported scenario must not report applied input")
        require(platform.get("rejected_input_results") is None or platform.get("rejected_input_results") == [],
                f"{prefix}: unsupported scenario must not report rejected input effects")
        platform_reports.append({
            "platform": platform_name,
            "status": "unsupported",
            "input_results": [],
            "stale_client_sequence_rejected": False,
            "max_latency_ms": None,
        })
    else:
        errors.append(f"{prefix}: status must be passed or unsupported")

macos = next((item for item in platform_reports if item["platform"] == "macos"), None)
require(isinstance(macos, dict) and macos.get("status") == "passed",
        "macos must pass pointer/keyboard input injection")

coverage = {
    item["platform"]: item["status"] in {"passed", "unsupported"}
    for item in platform_reports
}
report = {
    "script": "tools/scripts/remoteapp-input-injection-e2e.sh",
    "status": "failed" if errors else "passed",
    "errors": errors,
    "coverage": coverage,
    "platforms": platform_reports,
    "latency_threshold_ms": latency_threshold,
    "evidence_json": evidence_path,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# RemoteApp Input Injection E2E\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Evidence: `{evidence_path}`\n")
    f.write(f"- Latency threshold: `{latency_threshold}ms`\n")
    for item in platform_reports:
        f.write(f"- {item['platform']}: `{item['status']}`")
        if item["max_latency_ms"] is not None:
            f.write(f" max `{item['max_latency_ms']}ms`")
        f.write("\n")
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

subject = "easynet:///r/localhost/resource/device.macos/streams/display.primary"
session_id = "rd-input-macos-self-test"
abilities = [
    {"name": "remote_desktop.create_session", "subject_ura": subject},
    {"name": "remote_desktop.attach", "subject_ura": subject, "session_id": session_id},
    {"name": "remote_desktop.watch_events", "subject_ura": subject, "session_id": session_id},
    {"name": "remote_desktop.end_session", "subject_ura": subject, "session_id": session_id},
]
macos = {
    "platform": "macos",
    "status": "passed",
    "selected_resource_ura": subject,
    "session_id": session_id,
    "permission": {"accessibility_granted": True, "input_injection_granted": True},
    "consent_scope": "input_control",
    "input_scope": "display_global",
    "focus_validated": True,
    "coordinate_mapping_validated": True,
    "target_geometry_revision": 7,
    "source_only_proof": False,
    "policy_only": False,
    "abilities": abilities,
    "input_results": [
        {
            "kind": "pointer",
            "result": "input_applied",
            "event_type": "INPUT_FRAME_APPLIED",
            "client_sequence": 1,
            "client_sent_at_ms": 1787331000000,
            "host_received_at_ms": 1787331000010,
            "host_applied_at_ms": 1787331000019,
            "latency_ms": 19,
            "observed_effect": "pointer_position_changed",
            "coordinate_mapping": "target_geometry_revision_matched",
            "target_geometry_revision": 7,
        },
        {
            "kind": "keyboard",
            "result": "input_applied",
            "event_type": "INPUT_FRAME_APPLIED",
            "client_sequence": 2,
            "client_sent_at_ms": 1787331000100,
            "host_received_at_ms": 1787331000120,
            "host_applied_at_ms": 1787331000135,
            "latency_ms": 35,
            "observed_effect": "key_echo_observed",
            "key_code": "KeyA",
        },
    ],
    "rejected_input_results": [
        {
            "event_type": "INPUT_FRAME_REJECTED",
            "reason": "stale_client_sequence",
            "client_sequence": 1,
            "subject_ura": subject,
            "session_id": session_id,
        }
    ],
    "latency_summary": {"p95_ms": 35, "max_ms": 35},
    "terminal_receipt": {
        "terminal": True,
        "session_id": session_id,
        "reason_code": "input_injection_e2e_cleanup",
    },
}
unsupported = lambda platform: {
    "platform": platform,
    "status": "unsupported",
    "unsupported_state": "explicit_product_unsupported",
    "show_unsupported": True,
    "input_results": [],
    "rejected_input_results": [],
}
evidence = {
    "status": "passed",
    "proof_mode": "real_input_injection_matrix",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "latency_threshold_ms": 100,
    "platforms": [macos, unsupported("windows"), unsupported("linux")],
}
pathlib.Path(sys.argv[1]).write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  validate_evidence
  echo "remoteapp-input-injection-e2e self-test ok"
  exit 0
fi

if [[ "$MODE" != "run" ]]; then
  write_report "skipped" "set EASYNET_REMOTEAPP_INPUT_INJECTION_E2E=1 or pass --run"
  echo "[remoteapp-input-injection-e2e] skipped: $REPORT_MD"
  exit 0
fi

if [[ -n "$EVIDENCE_INPUT" ]]; then
  [[ -f "$EVIDENCE_INPUT" ]] || {
    write_report "failed" "evidence json does not exist: $EVIDENCE_INPUT"
    echo "[remoteapp-input-injection-e2e] missing evidence json: $EVIDENCE_INPUT" >&2
    exit 1
  }
  cp "$EVIDENCE_INPUT" "$EVIDENCE_JSON"
elif [[ -n "$RUNNER_CMD" ]]; then
  export EASYNET_REMOTEAPP_INPUT_INJECTION_EVIDENCE_JSON="$EVIDENCE_JSON"
  if ! bash -lc "$RUNNER_CMD" >"$RUNNER_STDOUT" 2>"$RUNNER_STDERR"; then
    write_report "failed" "runner command failed"
    echo "[remoteapp-input-injection-e2e] runner command failed" >&2
    cat "$RUNNER_STDERR" >&2 || true
    exit 1
  fi
  [[ -f "$EVIDENCE_JSON" ]] || {
    write_report "failed" "runner did not write evidence json"
    echo "[remoteapp-input-injection-e2e] runner did not write $EVIDENCE_JSON" >&2
    exit 1
  }
else
  write_report "failed" "--run requires --evidence-json or --runner-cmd"
  echo "[remoteapp-input-injection-e2e] --run requires --evidence-json or --runner-cmd" >&2
  exit 1
fi

validate_evidence
echo "[remoteapp-input-injection-e2e] PASS: $REPORT_MD"
