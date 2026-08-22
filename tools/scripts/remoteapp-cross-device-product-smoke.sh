#!/usr/bin/env bash
# RemoteApp cross-device product smoke gate.
#
# This gate intentionally composes existing Docker product-path E2Es instead
# of introducing a RemoteApp-specific daemon bypass. It proves a lower bound:
# two independently paired devices can route governed abilities through the Hub
# and can carry synthetic stream/bidi media frames with descriptor-bound
# receipts. It does not prove host OS window capture, real input injection,
# audio devices, or NAT/TURN deployment behavior.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
OUT_DIR="${EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-cross-device-product-smoke/$(date -u +%Y%m%d-%H%M%S)-$$}"
PROJECT_PREFIX="${EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_PROJECT_PREFIX:-easynet-remoteapp-cross-device}"
RUN=0
KEEP=0
BUILD=0

ROUTING_SMOKE="$SELF_DIR/docker-two-node-easyremote-cli-e2e.sh"
MEDIA_SMOKE="$SELF_DIR/docker-media-bidi-e2e.sh"

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/remoteapp-cross-device-product-smoke.sh --run [options]
  tools/scripts/remoteapp-cross-device-product-smoke.sh --self-test

Options:
  --run             Run cross-device routing and synthetic media smoke gates.
  --build           Allow child Docker E2Es to rebuild Linux/runtime images.
                    By default this gate reuses EASYNET_RUNTIME_IMAGE.
  --keep            Keep child containers/volumes on failure or success.
  --out-dir DIR     Report directory.
  --project-prefix  Docker Compose project prefix for child E2Es.
  --self-test       Validate this gate's source contract only.
  -h, --help        Show this help.

Environment:
  EASYNET_RUNTIME_IMAGE
                    Runtime image reused by the child E2E gates.

Evidence scope:
  This gate proves cross-device Hub routing and synthetic stream/bidi carrier
  behavior. It is not evidence for real macOS/Windows/Linux capture, host audio,
  pointer/keyboard injection, NAT/TURN deployment, or frontend browser rendering.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) RUN=1; shift ;;
    --build) BUILD=1; shift ;;
    --keep) KEEP=1; shift ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --project-prefix) PROJECT_PREFIX="${2:?missing value for --project-prefix}"; shift 2 ;;
    --self-test) RUN=self-test; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

write_report() {
  local status="$1"
  local reason="$2"
  python3 - "$OUT_DIR" "$status" "$reason" <<'PY'
import json
import pathlib
import sys

out_dir = pathlib.Path(sys.argv[1])
status = sys.argv[2]
reason = sys.argv[3]
out_dir.mkdir(parents=True, exist_ok=True)

steps = []
for name in ("cross-device-routing", "synthetic-media-bidi"):
    step_dir = out_dir / name
    result = {"name": name, "status": "not_run"}
    result_path = step_dir / "result.json"
    if result_path.exists():
        result.update(json.loads(result_path.read_text(encoding="utf-8")))
    report_path = step_dir / "report.json"
    if report_path.exists():
        report = json.loads(report_path.read_text(encoding="utf-8"))
        result["child_report"] = str(report_path)
        result["assertion_count"] = len(report.get("assertions") or {})
        result["failed_assertions"] = [
            key for key, value in (report.get("assertions") or {}).items()
            if value is not True
        ]
    stderr_path = step_dir / "stderr.txt"
    stderr_text = ""
    if stderr_path.exists():
        stderr_text = stderr_path.read_text(encoding="utf-8", errors="replace")
        result["stderr_excerpt"] = stderr_text[:4000]
    result["diagnostics"] = {
        "service_owner_projection_failed": (
            "advertise_service_abilities_prelude_failed" in stderr_text
            or "accepted_count=0, expected_count=5" in stderr_text
        ),
        "hub_routing_saw_provider": '"is_self": false' in stderr_text
        and '"online": true' in stderr_text,
        "synthetic_media_not_reached": (
            name == "synthetic-media-bidi" and result.get("status") == "not_run"
        ),
    }
    steps.append(result)

coverage = {
    "cross_device_hub_routing": any(
        step["name"] == "cross-device-routing" and step.get("status") == "passed"
        for step in steps
    ),
    "synthetic_stream_bidi_carrier": any(
        step["name"] == "synthetic-media-bidi" and step.get("status") == "passed"
        for step in steps
    ),
    "real_os_window_application_capture": False,
    "real_pointer_keyboard_injection": False,
    "real_audio_device_path": False,
    "nat_stun_turn_relay_deployment": False,
    "frontend_browser_rendering": False,
}
report = {
    "script": "tools/scripts/remoteapp-cross-device-product-smoke.sh",
    "status": status,
    "reason": reason,
    "steps": steps,
    "coverage": coverage,
    "non_claims": [
        "does not prove real OS window/application capture",
        "does not prove pointer/keyboard input injection",
        "does not prove host audio device capture/playback",
        "does not prove direct/STUN/TURN/EasyNet relay deployment",
        "does not prove frontend browser rendering or user interaction",
    ],
}
(out_dir / "report.json").write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
(out_dir / "report.md").write_text(
    "# RemoteApp Cross-Device Product Smoke\n\n"
    f"- Status: `{status}`\n"
    f"- Reason: `{reason}`\n"
    f"- Cross-device Hub routing: `{str(coverage['cross_device_hub_routing']).lower()}`\n"
    f"- Synthetic stream/bidi carrier: `{str(coverage['synthetic_stream_bidi_carrier']).lower()}`\n"
    "\nThis report is not product-complete RemoteApp evidence for real OS capture,\n"
    "input injection, host audio, NAT/TURN deployment, or frontend rendering.\n",
    encoding="utf-8",
)
PY
}

run_step() {
  local name="$1"
  shift
  local step_dir="$OUT_DIR/$name"
  mkdir -p "$step_dir"
  echo "[remoteapp-cross-device-product-smoke] running $name"
  if "$@" >"$step_dir/stdout.txt" 2>"$step_dir/stderr.txt"; then
    printf '{"status":"passed","name":"%s"}\n' "$name" >"$step_dir/result.json"
    return 0
  fi
  printf '{"status":"failed","name":"%s"}\n' "$name" >"$step_dir/result.json"
  echo "[remoteapp-cross-device-product-smoke] FAIL: $name" >&2
  cat "$step_dir/stderr.txt" >&2 || true
  write_report "failed" "step $name failed"
  return 1
}

build_child_args() {
  local project="$1"
  local out_dir="$2"
  CHILD_ARGS=(--project "$project" --out-dir "$out_dir")
  if [[ "$KEEP" == "1" ]]; then
    CHILD_ARGS+=(--keep)
  fi
  if [[ "$BUILD" != "1" ]]; then
    CHILD_ARGS+=(--skip-build)
  fi
}

if [[ "$RUN" == "self-test" ]]; then
  bash -n "$0"
  grep -q "docker-two-node-easyremote-cli-e2e.sh" "$0"
  grep -q "docker-media-bidi-e2e.sh" "$0"
  grep -q "cross_device_hub_routing" "$0"
  grep -q "synthetic_stream_bidi_carrier" "$0"
  grep -q "service_owner_projection_failed" "$0"
  grep -q "real_os_window_application_capture" "$0"
  grep -q "does not prove real OS window/application capture" "$0"
  grep -q -- "--skip-build" "$0"
  grep -q "write_report \"skipped\"" "$0"
  echo "remoteapp-cross-device-product-smoke self-test ok"
  exit 0
fi

mkdir -p "$OUT_DIR"

if [[ "$RUN" != "1" ]]; then
  write_report "skipped" "explicit --run was not provided"
  echo "[remoteapp-cross-device-product-smoke] SKIPPED: $OUT_DIR/report.md"
  exit 0
fi

[[ -x "$ROUTING_SMOKE" ]] || { echo "missing routing smoke: $ROUTING_SMOKE" >&2; exit 1; }
[[ -x "$MEDIA_SMOKE" ]] || { echo "missing media smoke: $MEDIA_SMOKE" >&2; exit 1; }

CHILD_ARGS=()
build_child_args "${PROJECT_PREFIX}-routing" "$OUT_DIR/cross-device-routing"
routing_args=("${CHILD_ARGS[@]}")
build_child_args "${PROJECT_PREFIX}-media" "$OUT_DIR/synthetic-media-bidi"
media_args=("${CHILD_ARGS[@]}")

run_step cross-device-routing "$ROUTING_SMOKE" "${routing_args[@]}"
cp "$OUT_DIR/cross-device-routing/report.json" "$OUT_DIR/cross-device-routing/child-report.json"

run_step synthetic-media-bidi "$MEDIA_SMOKE" "${media_args[@]}"
cp "$OUT_DIR/synthetic-media-bidi/report.json" "$OUT_DIR/synthetic-media-bidi/child-report.json"

write_report "passed" "cross-device routing and synthetic media smoke completed"
echo "[remoteapp-cross-device-product-smoke] PASS: $OUT_DIR/report.md"
