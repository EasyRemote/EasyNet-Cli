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

The --run path performs:
  1. Frontend TypeScript check.
  2. Frontend DeviceMediaAccess RemoteApp UI flow test.
  3. Host permission subject preflight with screen-capture permission granted.
  4. Host target picker freshness with a sentinel fixture.
  5. Host decoded-frame WebRTC E2E for window/application targets.
  6. Host view-only input safety for app/window targets.

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
report = {
    "script": "tools/scripts/frontend-remoteapp-product-flow-e2e.sh",
    "status": status,
    "reason": reason,
    "target_kind": target_kind,
    "evidence_contract": [
        "frontend TypeScript check",
        "DeviceMediaAccess RemoteApp UI flow",
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
    f"- Reason: `{reason}`\n",
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
  return 1
}

run_frontend_tsc() {
  (cd "$FRONTEND_ROOT" && npx tsc --noEmit)
}

run_frontend_ui_flow() {
  (cd "$FRONTEND_ROOT" && npm test -- src/components/easynet/DeviceMediaAccess.test.tsx)
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

if [[ "$SELF_TEST" -eq 1 ]]; then
  bash -n "$0"
  grep -q 'DeviceMediaAccess.test.tsx' "$0"
  grep -q 'npx tsc --noEmit' "$0"
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
  echo "[frontend-remoteapp-product-flow-e2e] missing frontend root: $FRONTEND_ROOT" >&2
  exit 1
}

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
