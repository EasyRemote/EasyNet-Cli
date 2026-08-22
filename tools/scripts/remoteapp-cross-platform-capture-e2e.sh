#!/usr/bin/env bash
# RemoteApp cross-platform capture E2E evidence verifier.
#
# Boundary:
# - This harness verifies evidence produced by real macOS/Windows/Linux host
#   runners for RemoteApp display/window/application capture.
# - It does not implement platform capture and does not simulate host windows.
#   A live pass requires either --evidence-json from an external runner or
#   --runner-cmd that writes the evidence JSON path provided through
#   EASYNET_REMOTEAPP_CROSS_PLATFORM_CAPTURE_EVIDENCE_JSON.
# - Self-test validates the evidence contract only; it is not product evidence.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

MODE=skip
SELF_TEST=0
OUT_DIR="${EASYNET_REMOTEAPP_CROSS_PLATFORM_CAPTURE_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-cross-platform-capture/$(date -u +%Y%m%d-%H%M%S)-$$}"
RUNNER_CMD="${EASYNET_REMOTEAPP_CROSS_PLATFORM_CAPTURE_RUNNER_CMD:-}"
EVIDENCE_INPUT="${EASYNET_REMOTEAPP_CROSS_PLATFORM_CAPTURE_EVIDENCE_JSON:-}"

usage() {
  cat <<'USAGE'
Usage:
  remoteapp-cross-platform-capture-e2e.sh --run --evidence-json PATH
  remoteapp-cross-platform-capture-e2e.sh --run --runner-cmd CMD
  remoteapp-cross-platform-capture-e2e.sh --self-test

Options:
  --run                 Verify real cross-platform capture evidence.
  --self-test           Validate the harness against synthetic positive evidence.
  --runner-cmd CMD      Command that drives real platform capture and writes
                        evidence to EASYNET_REMOTEAPP_CROSS_PLATFORM_CAPTURE_EVIDENCE_JSON.
  --evidence-json PATH  Existing evidence JSON emitted by real host runners.
  --out-dir DIR         Report directory.
  -h, --help            Show this help.

Environment:
  EASYNET_REMOTEAPP_CROSS_PLATFORM_CAPTURE_E2E=1
                        Equivalent to --run.

Evidence contract:
  The evidence JSON must prove a real cross-platform capture matrix, not
  source-only target binding checks. macOS must pass display/window/application
  capture. Windows and Linux must either pass those targets or report explicit
  product unsupported state without starting display fallback.

Non-claims:
  A skipped report or self-test does not prove cross-platform capture
  readiness. This harness verifies one capture artifact; input injection,
  network fallback, codec soak, frontend Browser/Tauri lifecycle, and
  cross-device product behavior still require their own evidence.
USAGE
}

if [[ "${EASYNET_REMOTEAPP_CROSS_PLATFORM_CAPTURE_E2E:-0}" == "1" ]]; then
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
coverage = {
    "macos": False,
    "windows": False,
    "linux": False,
}
report = {
    "script": "tools/scripts/remoteapp-cross-platform-capture-e2e.sh",
    "status": status,
    "reason": reason,
    "evidence_json": evidence_path,
    "coverage": coverage,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# RemoteApp Cross-Platform Capture E2E\n\n"
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
required_targets = {"display", "window", "application"}
expected_scope = {
    "display": "DisplaySurface",
    "window": "WindowSurface",
    "application": "AppSurface",
}
terminal_reasons = {"caller_ended", "user_cancelled", "capture_e2e_cleanup"}

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("proof_mode") == "real_cross_platform_capture_matrix",
        "proof_mode must be real_cross_platform_capture_matrix")
require(evidence.get("component_mock") is False,
        "component_mock must be false")
require(evidence.get("real_backend_runtime") is True,
        "real_backend_runtime must be true")
require(evidence.get("product_complete_claim") is False,
        "product_complete_claim must remain false")

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
    scenarios = platform.get("scenarios")
    require(isinstance(scenarios, list) and scenarios,
            f"{platform_name}: scenarios must be a non-empty list")
    scenario_by_target = {}
    if isinstance(scenarios, list):
        for scenario in scenarios:
            if not isinstance(scenario, dict):
                errors.append(f"{platform_name}: each scenario must be an object")
                continue
            target_kind = scenario.get("target_kind")
            if target_kind in scenario_by_target:
                errors.append(f"{platform_name}/{target_kind}: duplicate target scenario")
            scenario_by_target[target_kind] = scenario

    missing_targets = sorted(required_targets - set(scenario_by_target))
    require(not missing_targets, f"{platform_name}: missing target scenarios: " + ", ".join(missing_targets))

    passed_targets = []
    unsupported_targets = []
    for target_kind in sorted(required_targets):
        scenario = scenario_by_target.get(target_kind)
        if not isinstance(scenario, dict):
            continue
        prefix = f"{platform_name}/{target_kind}"
        require(scenario.get("target_kind") == target_kind,
                f"{prefix}: target_kind must match scenario key")
        status = scenario.get("status")
        if status == "passed":
            passed_targets.append(target_kind)
            subject_ura = scenario.get("selected_resource_ura")
            session_id = scenario.get("session_id")
            require(is_ura(subject_ura), f"{prefix}: selected_resource_ura must be canonical")
            require(isinstance(session_id, str) and session_id, f"{prefix}: session_id must be recorded")
            require(isinstance(scenario.get("capture_backend"), str) and scenario.get("capture_backend"),
                    f"{prefix}: capture_backend must be explicit")
            require(scenario.get("capture_scope") == expected_scope[target_kind],
                    f"{prefix}: capture_scope must be {expected_scope[target_kind]}")
            require(scenario.get("target_binding_exact") is True,
                    f"{prefix}: target_binding_exact must be true")
            require(scenario.get("source_only_proof") is False,
                    f"{prefix}: source_only_proof must be false")
            require(int(scenario.get("frames_rendered", 0)) > 0,
                    f"{prefix}: frames_rendered must be positive")
            require(int(scenario.get("duration_ms", 0)) > 0,
                    f"{prefix}: duration_ms must be positive")
            if target_kind in {"window", "application"}:
                require(scenario.get("first_display_capture_started") is False,
                        f"{prefix}: window/application capture must not start first-display fallback")
                require(scenario.get("display_fallback_used") is False,
                        f"{prefix}: display_fallback_used must be false")

            abilities = scenario.get("abilities")
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

            terminal = scenario.get("terminal_receipt")
            require(isinstance(terminal, dict), f"{prefix}: terminal_receipt must be visible")
            if not isinstance(terminal, dict):
                terminal = {}
            require(terminal.get("terminal") is True,
                    f"{prefix}: terminal_receipt.terminal must be true")
            require(terminal.get("session_id") == session_id,
                    f"{prefix}: terminal_receipt must bind session_id")
            require(terminal.get("reason_code") in terminal_reasons,
                    f"{prefix}: terminal_receipt.reason_code must be a known cleanup/end reason")
        elif status == "unsupported":
            unsupported_targets.append(target_kind)
            require(platform_name in {"windows", "linux"},
                    f"{prefix}: unsupported capture state is allowed only on Windows/Linux")
            require(scenario.get("unsupported_state") == "explicit_product_unsupported",
                    f"{prefix}: unsupported_state must be explicit_product_unsupported")
            require(scenario.get("show_unsupported") is True,
                    f"{prefix}: show_unsupported must be true")
            require(scenario.get("session_id") in {None, ""},
                    f"{prefix}: unsupported scenario must not create a capture session")
            require(int(scenario.get("frames_rendered", 0)) == 0,
                    f"{prefix}: unsupported scenario must not render frames")
            if target_kind in {"window", "application"}:
                require(scenario.get("first_display_capture_started") is False,
                        f"{prefix}: unsupported window/application must not start display fallback")
        else:
            errors.append(f"{prefix}: status must be passed or unsupported")

    if platform_name == "macos":
        require(set(passed_targets) == required_targets,
                "macos must pass display/window/application capture")
    platform_reports.append({
        "platform": platform_name,
        "passed_targets": sorted(passed_targets),
        "unsupported_targets": sorted(unsupported_targets),
    })

coverage = {
    item["platform"]: (
        set(item["passed_targets"]) == required_targets
        or (
            item["platform"] in {"windows", "linux"}
            and set(item["passed_targets"]) | set(item["unsupported_targets"]) == required_targets
        )
    )
    for item in platform_reports
}
report = {
    "script": "tools/scripts/remoteapp-cross-platform-capture-e2e.sh",
    "status": "failed" if errors else "passed",
    "errors": errors,
    "coverage": coverage,
    "platforms": platform_reports,
    "evidence_json": evidence_path,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# RemoteApp Cross-Platform Capture E2E\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Evidence: `{evidence_path}`\n")
    for platform_name, covered in sorted(coverage.items()):
        f.write(f"- {platform_name}: `{str(covered).lower()}`\n")
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

targets = {
    "display": "DisplaySurface",
    "window": "WindowSurface",
    "application": "AppSurface",
}

def passed(platform, target_kind):
    subject = f"easynet:///r/localhost/resource/device.{platform}/streams/{target_kind}.primary"
    session_id = f"rd-capture-{platform}-{target_kind}-self-test"
    return {
        "target_kind": target_kind,
        "status": "passed",
        "selected_resource_ura": subject,
        "session_id": session_id,
        "capture_backend": "macos.screencapturekit" if platform == "macos" else f"{platform}.native_capture",
        "capture_scope": targets[target_kind],
        "target_binding_exact": True,
        "source_only_proof": False,
        "frames_rendered": 3,
        "duration_ms": 1000,
        "first_display_capture_started": False,
        "display_fallback_used": False,
        "abilities": [
            {"name": "remote_desktop.create_session", "subject_ura": subject},
            {"name": "remote_desktop.attach", "subject_ura": subject, "session_id": session_id},
            {"name": "remote_desktop.watch_events", "subject_ura": subject, "session_id": session_id},
            {"name": "remote_desktop.end_session", "subject_ura": subject, "session_id": session_id},
        ],
        "terminal_receipt": {
            "terminal": True,
            "session_id": session_id,
            "reason_code": "capture_e2e_cleanup",
        },
    }

def unsupported(target_kind):
    return {
        "target_kind": target_kind,
        "status": "unsupported",
        "unsupported_state": "explicit_product_unsupported",
        "show_unsupported": True,
        "session_id": None,
        "frames_rendered": 0,
        "first_display_capture_started": False,
    }

evidence = {
    "status": "passed",
    "proof_mode": "real_cross_platform_capture_matrix",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "platforms": [
        {"platform": "macos", "scenarios": [passed("macos", target) for target in targets]},
        {"platform": "windows", "scenarios": [unsupported(target) for target in targets]},
        {"platform": "linux", "scenarios": [unsupported(target) for target in targets]},
    ],
}
pathlib.Path(sys.argv[1]).write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  validate_evidence
  echo "remoteapp-cross-platform-capture-e2e self-test ok"
  exit 0
fi

if [[ "$MODE" != "run" ]]; then
  write_report "skipped" "set EASYNET_REMOTEAPP_CROSS_PLATFORM_CAPTURE_E2E=1 or pass --run"
  echo "[remoteapp-cross-platform-capture-e2e] skipped: $REPORT_MD"
  exit 0
fi

if [[ -n "$EVIDENCE_INPUT" ]]; then
  [[ -f "$EVIDENCE_INPUT" ]] || {
    write_report "failed" "evidence json does not exist: $EVIDENCE_INPUT"
    echo "[remoteapp-cross-platform-capture-e2e] missing evidence json: $EVIDENCE_INPUT" >&2
    exit 1
  }
  cp "$EVIDENCE_INPUT" "$EVIDENCE_JSON"
elif [[ -n "$RUNNER_CMD" ]]; then
  export EASYNET_REMOTEAPP_CROSS_PLATFORM_CAPTURE_EVIDENCE_JSON="$EVIDENCE_JSON"
  if ! bash -lc "$RUNNER_CMD" >"$RUNNER_STDOUT" 2>"$RUNNER_STDERR"; then
    write_report "failed" "runner command failed"
    echo "[remoteapp-cross-platform-capture-e2e] runner command failed" >&2
    cat "$RUNNER_STDERR" >&2 || true
    exit 1
  fi
  [[ -f "$EVIDENCE_JSON" ]] || {
    write_report "failed" "runner did not write evidence json"
    echo "[remoteapp-cross-platform-capture-e2e] runner did not write $EVIDENCE_JSON" >&2
    exit 1
  }
else
  write_report "failed" "--run requires --evidence-json or --runner-cmd"
  echo "[remoteapp-cross-platform-capture-e2e] --run requires --evidence-json or --runner-cmd" >&2
  exit 1
fi

validate_evidence
echo "[remoteapp-cross-platform-capture-e2e] PASS: $REPORT_MD"
