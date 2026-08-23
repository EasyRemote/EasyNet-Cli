#!/usr/bin/env bash
# Frontend RemoteApp product-flow E2E harness.
#
# This is the product-flow entrypoint for RemoteApp. It deliberately combines
# frontend surface coverage, cross-device routing evidence, and host daemon
# evidence instead of treating a component mock or a host-only CLI probe as full
# product proof.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
DEFAULT_FRONTEND_ROOT="$REPO_ROOT/../EasyNet/Frontend"
FRONTEND_ROOT="${EASYNET_FRONTEND_ROOT:-$DEFAULT_FRONTEND_ROOT}"
OUT_DIR="${EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_OUT_DIR:-$REPO_ROOT/target/e2e/frontend-remoteapp-product-flow/$(date -u +%Y%m%d-%H%M%S)-$$}"
RUN="${EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E:-0}"
SELF_TEST=0
TARGET_KIND="${EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_TARGET_KIND:-both}"

PERMISSION_SUBJECT="$SELF_DIR/host-remoteapp-permission-subject-e2e.sh"
TARGET_FRESHNESS="$SELF_DIR/host-remoteapp-target-picker-freshness-e2e.sh"
DECODED_FRAME="$SELF_DIR/host-remoteapp-decoded-frame-e2e.sh"
VIEW_ONLY_INPUT="$SELF_DIR/host-remoteapp-view-only-input-safety-e2e.sh"
HUB_API_PREFLIGHT="$SELF_DIR/hub-api-readiness-preflight.sh"
BROWSER_LIFECYCLE="$SELF_DIR/frontend-remoteapp-browser-lifecycle-e2e.sh"
CROSS_DEVICE_SMOKE="$SELF_DIR/remoteapp-cross-device-product-smoke.sh"
BROWSER_LIFECYCLE_EVIDENCE_JSON="${EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_BROWSER_LIFECYCLE_EVIDENCE_JSON:-${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON:-}}"
BROWSER_LIFECYCLE_RUNNER_CMD="${EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_BROWSER_LIFECYCLE_RUNNER_CMD:-${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_RUNNER_CMD:-}}"
BROWSER_LIFECYCLE_FRONTEND_URL="${EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_BROWSER_LIFECYCLE_FRONTEND_URL:-${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL:-}}"
BROWSER_LIFECYCLE_SURFACE="${EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_BROWSER_LIFECYCLE_SURFACE:-${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_SURFACE:-browser}}"
CROSS_DEVICE_SMOKE_EVIDENCE_JSON="${EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_CROSS_DEVICE_SMOKE_EVIDENCE_JSON:-${EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_EVIDENCE_JSON:-}}"
CROSS_DEVICE_SMOKE_RUN="${EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_CROSS_DEVICE_SMOKE_RUN:-0}"
CROSS_DEVICE_SMOKE_BUILD="${EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_CROSS_DEVICE_SMOKE_BUILD:-0}"
CROSS_DEVICE_SMOKE_KEEP="${EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_CROSS_DEVICE_SMOKE_KEEP:-0}"
CROSS_DEVICE_SMOKE_PROJECT_PREFIX="${EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_CROSS_DEVICE_SMOKE_PROJECT_PREFIX:-}"

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/frontend-remoteapp-product-flow-e2e.sh --run [options]
  tools/scripts/frontend-remoteapp-product-flow-e2e.sh --self-test

Options:
  --run                 Run frontend and host RemoteApp product-flow evidence.
  --target-kind KIND    window, application, or both. Default: both.
  --out-dir DIR         Report directory.
  --self-test           Validate harness structure without requiring OS/daemon permissions.
  -h, --help            Show this help.

Environment:
  EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E=1
                        Equivalent to --run.
  EASYNET_FRONTEND_ROOT Path to EasyNet/Frontend. Defaults to ../EasyNet/Frontend.
  EASYNET_REMOTEAPP_EASYNET_BIN
                        Optional easynet binary override for daemon preflight
                        and delegated host E2E harnesses.

The --run path performs:
  1. Hub API readiness preflight.
  2. Product runtime readiness preflight for daemon control/invocation.
  3. Frontend TypeScript check.
  4. Frontend DeviceMediaAccess RemoteApp UI flow test.
  5. Real Browser/Tauri RemoteApp lifecycle verifier using an external runner
     or pre-existing evidence JSON.
  6. Cross-device product smoke evidence with distinct caller/provider device URAs.
  7. Host permission subject preflight with screen-capture permission granted.
  8. Host target picker freshness with a sentinel fixture.
  9. Host decoded-frame WebRTC E2E for window/application targets.
  10. Host view-only input safety for app/window targets.

Set EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_BROWSER_LIFECYCLE_EVIDENCE_JSON to
an existing Browser/Tauri lifecycle evidence artifact, or set
EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_BROWSER_LIFECYCLE_RUNNER_CMD together
with EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_BROWSER_LIFECYCLE_FRONTEND_URL.

Set EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_CROSS_DEVICE_SMOKE_EVIDENCE_JSON to
an existing cross-device smoke report, or set
EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_CROSS_DEVICE_SMOKE_RUN=1 to run
remoteapp-cross-device-product-smoke.sh as part of the product flow.

This harness still does not claim product completion by itself; it produces one
bounded E2E evidence bundle for the frontend + daemon + cross-device + host
RemoteApp flow.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) RUN=1; shift ;;
    --target-kind)
      case "${2:?missing value for --target-kind}" in
        window|application|both) TARGET_KIND="$2" ;;
        *) echo "invalid target kind: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --self-test) SELF_TEST=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

write_json_report() {
  local status="$1"
  local reason="$2"
  python3 - "$OUT_DIR" "$status" "$reason" "$TARGET_KIND" <<'PY'
import json
import pathlib
import sys

out_dir = pathlib.Path(sys.argv[1])
status = sys.argv[2]
reason = sys.argv[3]
target_kind = sys.argv[4]
out_dir.mkdir(parents=True, exist_ok=True)
steps = []
failed_step = None
failed_step_stderr = None
step_order = [
    "hub-api-readiness-preflight",
    "product-runtime-readiness-preflight",
    "frontend-typecheck",
    "frontend-remoteapp-ui-flow",
    "frontend-browser-lifecycle",
    "cross-device-product-smoke",
    "host-permission-subject",
    "host-target-picker-freshness",
    "host-decoded-frame-window",
    "host-decoded-frame-application",
    "host-view-only-input-window",
    "host-view-only-input-application",
]
result_paths = {path.parent.name: path for path in out_dir.glob("*/result.json")}
ordered_result_paths = [
    result_paths.pop(name)
    for name in step_order
    if name in result_paths
]
ordered_result_paths.extend(path for _, path in sorted(result_paths.items()))
for result_path in ordered_result_paths:
    try:
        result = json.loads(result_path.read_text(encoding="utf-8"))
    except Exception as exc:  # pragma: no cover - defensive report path
        result = {
            "name": result_path.parent.name,
            "status": "invalid",
            "error": f"invalid result json: {exc}",
        }
    stderr_path = result_path.parent / "stderr.txt"
    stderr_excerpt = ""
    if stderr_path.exists():
        stderr_excerpt = stderr_path.read_text(encoding="utf-8", errors="replace")[:4000]
    result["stderr_excerpt"] = stderr_excerpt
    steps.append(result)
    if failed_step is None and result.get("status") != "passed":
        failed_step = result.get("name") or result_path.parent.name
        failed_step_stderr = stderr_excerpt
report = {
    "script": "tools/scripts/frontend-remoteapp-product-flow-e2e.sh",
    "status": status,
    "reason": reason,
    "target_kind": target_kind,
    "step_order": step_order,
    "failed_step": failed_step,
    "failed_step_stderr": failed_step_stderr,
    "steps": steps,
    "evidence_contract": [
        "frontend TypeScript check",
        "DeviceMediaAccess RemoteApp UI flow",
        "Browser/Tauri RemoteApp lifecycle evidence",
        "cross-device product smoke with distinct device URAs",
        "hub api readiness preflight",
        "product runtime readiness preflight",
        "host permission subject preflight",
        "host target picker freshness",
        "host decoded-frame WebRTC",
        "host view-only input safety",
    ],
}
(out_dir / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
(out_dir / "report.md").write_text(
    "# Frontend RemoteApp Product Flow E2E\n\n"
    f"- Status: `{status}`\n"
    f"- Target kind: `{target_kind}`\n"
    f"- Reason: `{reason}`\n"
    f"- Failed step: `{failed_step or ''}`\n",
    encoding="utf-8",
)
PY
}

run_step() {
  local name="$1"
  shift
  local step_dir="$OUT_DIR/$name"
  mkdir -p "$step_dir"
  echo "[frontend-remoteapp-product-flow-e2e] running $name"
  if "$@" >"$step_dir/stdout.txt" 2>"$step_dir/stderr.txt"; then
    printf '{"status":"passed","name":"%s"}\n' "$name" >"$step_dir/result.json"
    return 0
  fi
  printf '{"status":"failed","name":"%s"}\n' "$name" >"$step_dir/result.json"
  echo "[frontend-remoteapp-product-flow-e2e] FAIL: $name" >&2
  cat "$step_dir/stderr.txt" >&2 || true
  write_json_report "failed" "step $name failed"
  return 1
}

run_frontend_tsc() {
  (cd "$FRONTEND_ROOT" && npx tsc --noEmit)
}

run_frontend_ui_flow() {
  (cd "$FRONTEND_ROOT" && npm test -- src/components/easynet/DeviceMediaAccess.test.tsx)
}

run_frontend_browser_lifecycle() {
  local args=("--run" "--surface" "$BROWSER_LIFECYCLE_SURFACE" "--out-dir" "$OUT_DIR/frontend-browser-lifecycle")
  if [[ -n "$BROWSER_LIFECYCLE_EVIDENCE_JSON" ]]; then
    args+=("--evidence-json" "$BROWSER_LIFECYCLE_EVIDENCE_JSON")
  elif [[ -n "$BROWSER_LIFECYCLE_RUNNER_CMD" ]]; then
    if [[ -z "$BROWSER_LIFECYCLE_FRONTEND_URL" ]]; then
      echo "frontend Browser/Tauri lifecycle runner requires EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_BROWSER_LIFECYCLE_FRONTEND_URL" >&2
      return 64
    fi
    args+=("--runner-cmd" "$BROWSER_LIFECYCLE_RUNNER_CMD" "--frontend-url" "$BROWSER_LIFECYCLE_FRONTEND_URL")
  else
    echo "frontend Browser/Tauri lifecycle evidence is required: set EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_BROWSER_LIFECYCLE_EVIDENCE_JSON or EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_BROWSER_LIFECYCLE_RUNNER_CMD" >&2
    return 64
  fi
  "$BROWSER_LIFECYCLE" "${args[@]}"
}

validate_cross_device_smoke_report() {
  local report_json="$1"
  python3 - "$report_json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
topology = report.get("topology") if isinstance(report, dict) else None
coverage = report.get("coverage") if isinstance(report, dict) else None
errors = []
if report.get("status") != "passed":
    errors.append(f"cross-device smoke status is {report.get('status')!r}, expected 'passed'")
if report.get("product_complete_claim") is not False:
    errors.append("cross-device smoke must not claim product completion")
if not isinstance(topology, dict):
    errors.append("cross-device smoke report is missing topology")
else:
    if topology.get("requires_distinct_devices") is not True:
        errors.append("cross-device smoke topology must require distinct devices")
    if topology.get("distinct_device_uras_observed") is not True:
        errors.append("distinct_device_uras_observed is not true")
    if topology.get("local_provider_boundary_only") is not False:
        errors.append("local_provider_boundary_only is not false")
if not isinstance(coverage, dict):
    errors.append("cross-device smoke report is missing coverage")
else:
    if coverage.get("cross_device_hub_routing") is not True:
        errors.append("cross_device_hub_routing is not true")
    if coverage.get("synthetic_stream_bidi_carrier") is not True:
        errors.append("synthetic_stream_bidi_carrier is not true")
    if coverage.get("distinct_device_uras_observed") is not True:
        errors.append("coverage distinct_device_uras_observed is not true")
    if coverage.get("local_provider_boundary_only") is not False:
        errors.append("coverage local_provider_boundary_only is not false")
if errors:
    for error in errors:
        print(error, file=sys.stderr)
    raise SystemExit(1)
print("cross-device smoke evidence ok")
PY
}

run_cross_device_product_smoke() {
  local step_dir="$OUT_DIR/cross-device-product-smoke"
  local artifact_dir="$step_dir/artifact"
  mkdir -p "$artifact_dir"
  if [[ -n "$CROSS_DEVICE_SMOKE_EVIDENCE_JSON" ]]; then
    cp "$CROSS_DEVICE_SMOKE_EVIDENCE_JSON" "$step_dir/evidence-report.json"
    validate_cross_device_smoke_report "$step_dir/evidence-report.json"
    return 0
  fi
  if [[ "$CROSS_DEVICE_SMOKE_RUN" == "1" ]]; then
    local args=("--run" "--out-dir" "$artifact_dir")
    if [[ "$CROSS_DEVICE_SMOKE_BUILD" == "1" ]]; then
      args+=("--build")
    fi
    if [[ "$CROSS_DEVICE_SMOKE_KEEP" == "1" ]]; then
      args+=("--keep")
    fi
    if [[ -n "$CROSS_DEVICE_SMOKE_PROJECT_PREFIX" ]]; then
      args+=("--project-prefix" "$CROSS_DEVICE_SMOKE_PROJECT_PREFIX")
    fi
    "$CROSS_DEVICE_SMOKE" "${args[@]}"
    cp "$artifact_dir/report.json" "$step_dir/evidence-report.json"
    validate_cross_device_smoke_report "$step_dir/evidence-report.json"
    return 0
  fi
  echo "cross-device product smoke evidence is required: set EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_CROSS_DEVICE_SMOKE_EVIDENCE_JSON or EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_CROSS_DEVICE_SMOKE_RUN=1" >&2
  return 64
}

run_with_timeout() {
  local timeout_sec="$1"
  shift
  python3 - "$timeout_sec" "$@" <<'PY'
import subprocess
import sys

timeout_sec = float(sys.argv[1])
cmd = sys.argv[2:]
try:
    completed = subprocess.run(cmd, timeout=timeout_sec)
except subprocess.TimeoutExpired:
    print(
        f"command timed out after {timeout_sec:g}s: {' '.join(cmd)}",
        file=sys.stderr,
    )
    raise SystemExit(124)
raise SystemExit(completed.returncode)
PY
}

run_easynet() {
  local timeout_sec="${EASYNET_REMOTEAPP_EASYNET_COMMAND_TIMEOUT_SEC:-45}"
  if [[ -n "${EASYNET_REMOTEAPP_EASYNET_BIN:-}" ]]; then
    run_with_timeout "$timeout_sec" "$EASYNET_REMOTEAPP_EASYNET_BIN" "$@"
  elif [[ -x "$REPO_ROOT/target/debug/easynet" ]]; then
    run_with_timeout "$timeout_sec" "$REPO_ROOT/target/debug/easynet" "$@"
  else
    run_with_timeout "$timeout_sec" cargo run --quiet --bin easynet -- "$@"
  fi
}

run_product_runtime_readiness_preflight() {
  local step_dir="$OUT_DIR/product-runtime-readiness-preflight"
  local status_json="$step_dir/runtime-status.json"
  mkdir -p "$step_dir"
  run_easynet runtime status --json >"$status_json"
  python3 - "$status_json" <<'PY'
import json
import pathlib
import sys

status_path = pathlib.Path(sys.argv[1])
status = json.loads(status_path.read_text(encoding="utf-8"))
daemon = status.get("daemon") if isinstance(status, dict) else None
connection = status.get("connection") if isinstance(status, dict) else None
runtime_status = status.get("runtime_status") if isinstance(status, dict) else None
errors = []
hub_api_endpoint = None
if not isinstance(daemon, dict):
    errors.append("runtime status did not include daemon object")
else:
    if daemon.get("control_accepting") is not True:
        errors.append("daemon.control_accepting is not true")
    if daemon.get("invocation_accepting") is not True:
        errors.append("daemon.invocation_accepting is not true")
    if daemon.get("pid_alive") is not True:
        errors.append("daemon.pid_alive is not true")
if isinstance(connection, dict):
    hub_api_endpoint = connection.get("hub_api_endpoint")
    failure = connection.get("failure")
    if isinstance(failure, dict):
        errors.append(
            "connection.failure="
            f"{failure.get('code')}: {failure.get('message')}"
        )
if hub_api_endpoint:
    print(f"hub_api_endpoint={hub_api_endpoint}", file=sys.stderr)
if errors:
    print(f"runtime_status={runtime_status}", file=sys.stderr)
    for error in errors:
        print(error, file=sys.stderr)
    raise SystemExit(1)
print("product runtime readiness preflight ok")
PY
}

run_decoded_frame_kind() {
  local kind="$1"
  "$DECODED_FRAME" \
    --run \
    --sentinel-fixture \
    --pre-media-resource-refresh \
    --target-kind "$kind" \
    --out-dir "$OUT_DIR/host-decoded-frame-$kind"
}

run_view_only_input_kind() {
  local kind="$1"
  "$VIEW_ONLY_INPUT" \
    --run \
    --sentinel-fixture \
    --target-kind "$kind" \
    --out-dir "$OUT_DIR/host-view-only-input-$kind"
}

run_hub_api_readiness_preflight() {
  "$HUB_API_PREFLIGHT" --run --out-dir "$OUT_DIR/hub-api-readiness-preflight"
}

if [[ "$SELF_TEST" -eq 1 ]]; then
  bash -n "$0"
  grep -q 'hub-api-readiness-preflight.sh' "$0"
  grep -q 'run_hub_api_readiness_preflight' "$0"
  grep -q 'DeviceMediaAccess.test.tsx' "$0"
  grep -q 'npx tsc --noEmit' "$0"
  grep -q 'frontend-remoteapp-browser-lifecycle-e2e.sh' "$0"
  grep -q 'remoteapp-cross-device-product-smoke.sh' "$0"
  grep -q 'run_frontend_browser_lifecycle' "$0"
  grep -q 'run_cross_device_product_smoke' "$0"
  grep -q 'EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_BROWSER_LIFECYCLE_EVIDENCE_JSON' "$0"
  grep -q 'EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_BROWSER_LIFECYCLE_RUNNER_CMD' "$0"
  grep -q 'frontend Browser/Tauri lifecycle evidence is required' "$0"
  grep -q 'EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_CROSS_DEVICE_SMOKE_EVIDENCE_JSON' "$0"
  grep -q 'EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_CROSS_DEVICE_SMOKE_RUN' "$0"
  grep -q 'cross-device product smoke evidence is required' "$0"
  grep -q 'distinct_device_uras_observed is not true' "$0"
  self_test_tmp="$(mktemp -d)"
  trap 'rm -rf "$self_test_tmp"' EXIT
  python3 - "$self_test_tmp/cross-device-good.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
path.write_text(json.dumps({
    "status": "passed",
    "product_complete_claim": False,
    "topology": {
        "requires_distinct_devices": True,
        "distinct_device_uras_observed": True,
        "local_provider_boundary_only": False,
    },
    "coverage": {
        "cross_device_hub_routing": True,
        "synthetic_stream_bidi_carrier": True,
        "distinct_device_uras_observed": True,
        "local_provider_boundary_only": False,
    },
}) + "\n", encoding="utf-8")
PY
  validate_cross_device_smoke_report "$self_test_tmp/cross-device-good.json" >/dev/null
  python3 - "$self_test_tmp/cross-device-local-only.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
path.write_text(json.dumps({
    "status": "passed",
    "product_complete_claim": False,
    "topology": {
        "requires_distinct_devices": True,
        "distinct_device_uras_observed": False,
        "local_provider_boundary_only": True,
    },
    "coverage": {
        "cross_device_hub_routing": True,
        "synthetic_stream_bidi_carrier": True,
        "distinct_device_uras_observed": False,
        "local_provider_boundary_only": True,
    },
}) + "\n", encoding="utf-8")
PY
  if validate_cross_device_smoke_report "$self_test_tmp/cross-device-local-only.json" >/dev/null 2>&1; then
    echo "self-test accepted local-provider-only cross-device smoke evidence" >&2
    exit 1
  fi
  grep -q 'run_product_runtime_readiness_preflight' "$0"
  grep -q 'daemon.invocation_accepting is not true' "$0"
  grep -q 'product-runtime-readiness-preflight' "$0"
  grep -q 'host-remoteapp-permission-subject-e2e.sh' "$0"
  grep -q -- '--require-screen-capture-granted' "$0"
  grep -q 'host-remoteapp-target-picker-freshness-e2e.sh' "$0"
  grep -q 'host-remoteapp-decoded-frame-e2e.sh' "$0"
  grep -q -- '--pre-media-resource-refresh' "$0"
  grep -q 'host-remoteapp-view-only-input-safety-e2e.sh' "$0"
  grep -q 'EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E' "$0"
  echo "frontend-remoteapp-product-flow-e2e self-test ok"
  exit 0
fi

mkdir -p "$OUT_DIR"

if [[ "$RUN" != "1" ]]; then
  write_json_report "skipped" "set EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E=1 or pass --run"
  echo "[frontend-remoteapp-product-flow-e2e] skipped: $OUT_DIR/report.md"
  exit 0
fi

[[ -d "$FRONTEND_ROOT" ]] || {
  mkdir -p "$OUT_DIR/frontend-root"
  printf '[frontend-remoteapp-product-flow-e2e] missing frontend root: %s\n' "$FRONTEND_ROOT" >"$OUT_DIR/frontend-root/stderr.txt"
  printf '{"status":"failed","name":"frontend-root"}\n' >"$OUT_DIR/frontend-root/result.json"
  write_json_report "failed" "missing frontend root"
  cat "$OUT_DIR/frontend-root/stderr.txt" >&2
  exit 1
}

run_step hub-api-readiness-preflight run_hub_api_readiness_preflight
run_step product-runtime-readiness-preflight run_product_runtime_readiness_preflight
run_step frontend-typecheck run_frontend_tsc
run_step frontend-remoteapp-ui-flow run_frontend_ui_flow
run_step frontend-browser-lifecycle run_frontend_browser_lifecycle
run_step cross-device-product-smoke run_cross_device_product_smoke
run_step host-permission-subject "$PERMISSION_SUBJECT" --run --require-screen-capture-granted --out-dir "$OUT_DIR/host-permission-subject"
run_step host-target-picker-freshness "$TARGET_FRESHNESS" --run --sentinel-fixture --target-kind window --out-dir "$OUT_DIR/host-target-picker-freshness"

case "$TARGET_KIND" in
  window)
    run_step host-decoded-frame-window run_decoded_frame_kind window
    run_step host-view-only-input-window run_view_only_input_kind window
    ;;
  application)
    run_step host-decoded-frame-application run_decoded_frame_kind application
    run_step host-view-only-input-application run_view_only_input_kind application
    ;;
  both)
    run_step host-decoded-frame-window run_decoded_frame_kind window
    run_step host-decoded-frame-application run_decoded_frame_kind application
    run_step host-view-only-input-window run_view_only_input_kind window
    run_step host-view-only-input-application run_view_only_input_kind application
    ;;
esac

write_json_report "passed" "frontend, cross-device, and host RemoteApp product-flow evidence completed"
echo "[frontend-remoteapp-product-flow-e2e] PASS: $OUT_DIR/report.md"
