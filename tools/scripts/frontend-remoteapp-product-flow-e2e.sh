#!/usr/bin/env bash
# Frontend RemoteApp product-flow E2E harness.
#
# This is the product-flow entrypoint for RemoteApp. It deliberately combines
# frontend surface coverage with host daemon evidence instead of treating a
# component mock or a host-only CLI probe as full product proof.

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
  5. Host permission subject preflight with screen-capture permission granted.
  6. Host target picker freshness with a sentinel fixture.
  7. Host decoded-frame WebRTC E2E for window/application targets.
  8. Host view-only input safety for app/window targets.

This harness still does not claim product completion by itself; it produces one
bounded E2E evidence bundle for the frontend + daemon + host RemoteApp flow.
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

write_json_report "passed" "frontend and host RemoteApp product-flow evidence completed"
echo "[frontend-remoteapp-product-flow-e2e] PASS: $OUT_DIR/report.md"
