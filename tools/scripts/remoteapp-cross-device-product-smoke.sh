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
MIN_FREE_KIB="${EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_MIN_FREE_KIB:-2097152}"
DOCKER_INFO_TIMEOUT_SECONDS="${EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_DOCKER_INFO_TIMEOUT_SECONDS:-20}"
STEP_TIMEOUT_SECONDS="${EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_STEP_TIMEOUT_SECONDS:-900}"
RUNTIME_IMAGE="${EASYNET_RUNTIME_IMAGE:-${EASYNET_HUB_IMAGE:-easynet/hub-e2e:local}}"
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
  EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_MIN_FREE_KIB
                    Minimum free KiB required on the report filesystem before
                    running child Docker E2Es. Defaults to 2097152 (2 GiB).
  EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_DOCKER_INFO_TIMEOUT_SECONDS
                    Timeout for the Docker readiness probe. Defaults to 20.
  EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_STEP_TIMEOUT_SECONDS
                    Timeout for each child E2E step. Defaults to 900.

Evidence scope:
  This gate proves cross-device Hub routing and synthetic stream/bidi carrier
  behavior. It is not evidence for real macOS/Windows/Linux capture, host audio,
  pointer/keyboard injection, NAT/TURN deployment, or frontend browser rendering.
  A completed run must observe distinct caller/provider device URAs; same-device
  or local-provider-only topology is a failed cross-device smoke, not a pass
  with caveats.
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
  local source_revision source_dirty runtime_image_id runtime_image_created
  source_revision="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || printf 'unknown')"
  if [[ -n "$(git -C "$REPO_ROOT" status --porcelain 2>/dev/null || true)" ]]; then
    source_dirty=true
  else
    source_dirty=false
  fi
  runtime_image_id="$(docker image inspect "$RUNTIME_IMAGE" --format '{{.Id}}' 2>/dev/null || true)"
  runtime_image_created="$(docker image inspect "$RUNTIME_IMAGE" --format '{{.Created}}' 2>/dev/null || true)"
  python3 - "$OUT_DIR" "$status" "$reason" "$source_revision" "$source_dirty" \
    "$RUNTIME_IMAGE" "$runtime_image_id" "$runtime_image_created" "$BUILD" <<'PY'
import json
import pathlib
import sys

out_dir = pathlib.Path(sys.argv[1])
status = sys.argv[2]
reason = sys.argv[3]
source_revision = sys.argv[4]
source_dirty = sys.argv[5].lower() == "true"
runtime_image = sys.argv[6]
runtime_image_id = sys.argv[7] or None
runtime_image_created = sys.argv[8] or None
build_requested = sys.argv[9] == "1"
out_dir.mkdir(parents=True, exist_ok=True)

def read_json(path):
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return None

steps = []
observed_device_pairs = []
for name in ("cross-device-routing", "synthetic-media-bidi"):
    step_dir = out_dir / name
    result = {"name": name, "status": "not_run"}
    result_path = step_dir / "result.json"
    result_doc = read_json(result_path)
    if isinstance(result_doc, dict):
        result.update(result_doc)
    report_path = step_dir / "report.json"
    child_report = None
    if report_path.exists():
        report = read_json(report_path)
        child_report = report if isinstance(report, dict) else None
        result["child_report"] = str(report_path)
        result["assertion_count"] = len((child_report or {}).get("assertions") or {})
        result["failed_assertions"] = [
            key for key, value in ((child_report or {}).get("assertions") or {}).items()
            if value is not True
        ]
    topology = (child_report or {}).get("topology") if isinstance(child_report, dict) else None
    if isinstance(topology, dict):
        caller_ura = topology.get("caller_ura")
        provider_ura = topology.get("provider_ura")
        pair = {
            "step": name,
            "caller_ura": caller_ura if isinstance(caller_ura, str) else None,
            "provider_ura": provider_ura if isinstance(provider_ura, str) else None,
            "caller_node": topology.get("caller_node") if isinstance(topology.get("caller_node"), str) else None,
            "provider_node": topology.get("provider_node") if isinstance(topology.get("provider_node"), str) else None,
        }
        pair["distinct_device_uras"] = bool(
            pair["caller_ura"]
            and pair["provider_ura"]
            and pair["caller_ura"] != pair["provider_ura"]
        )
        observed_device_pairs.append(pair)
        result["topology"] = pair
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

distinct_device_uras_observed = any(pair["distinct_device_uras"] for pair in observed_device_pairs)
local_provider_boundary_only = not distinct_device_uras_observed
effective_status = status
effective_reason = reason
if status == "passed" and local_provider_boundary_only:
    effective_status = "failed"
    effective_reason = (
        "distinct device URAs were not observed; "
        "local_provider_boundary_only=true"
    )
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
    "distinct_device_uras_observed": distinct_device_uras_observed,
    "local_provider_boundary_only": local_provider_boundary_only,
}
report = {
    "script": "tools/scripts/remoteapp-cross-device-product-smoke.sh",
    "status": effective_status,
    "reason": effective_reason,
    "product_complete_claim": False,
    "source": {
        "revision": source_revision,
        "dirty": source_dirty,
    },
    "runtime": {
        "image": runtime_image,
        "image_id": runtime_image_id,
        "image_created": runtime_image_created,
        "build_requested": build_requested,
    },
    "topology": {
        "requires_distinct_devices": True,
        "observed_device_pairs": observed_device_pairs,
        "distinct_device_uras_observed": distinct_device_uras_observed,
        "local_provider_boundary_only": local_provider_boundary_only,
    },
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
    f"- Status: `{effective_status}`\n"
    f"- Reason: `{effective_reason}`\n"
    f"- Source revision: `{source_revision}`\n"
    f"- Source dirty: `{str(source_dirty).lower()}`\n"
    f"- Runtime image: `{runtime_image}`\n"
    f"- Runtime image id: `{runtime_image_id or 'unknown'}`\n"
    f"- Runtime image created: `{runtime_image_created or 'unknown'}`\n"
    f"- Runtime image build requested: `{str(build_requested).lower()}`\n"
    f"- Distinct device URAs observed: `{str(distinct_device_uras_observed).lower()}`\n"
    f"- Local provider boundary only: `{str(local_provider_boundary_only).lower()}`\n"
    f"- Cross-device Hub routing: `{str(coverage['cross_device_hub_routing']).lower()}`\n"
    f"- Synthetic stream/bidi carrier: `{str(coverage['synthetic_stream_bidi_carrier']).lower()}`\n"
    "\nThis report is not product-complete RemoteApp evidence for real OS capture,\n"
    "input injection, host audio, NAT/TURN deployment, or frontend rendering.\n",
    encoding="utf-8",
)
PY
}

fail_preflight() {
  local reason="$1"
  echo "[remoteapp-cross-device-product-smoke] FAIL: $reason" >&2
  write_report "failed" "$reason"
  exit 1
}

require_free_space() {
  python3 - "$OUT_DIR" "$MIN_FREE_KIB" <<'PY'
import os
import pathlib
import sys

out_dir = pathlib.Path(sys.argv[1])
min_free_kib = int(sys.argv[2])
probe_dir = out_dir if out_dir.exists() else out_dir.parent
while not probe_dir.exists() and probe_dir != probe_dir.parent:
    probe_dir = probe_dir.parent
stat = os.statvfs(probe_dir)
free_kib = (stat.f_bavail * stat.f_frsize) // 1024
if free_kib < min_free_kib:
    print(
        "insufficient free space for cross-device smoke reports "
        f"(path={probe_dir}, free_kib={free_kib}, required_kib={min_free_kib})"
    )
    raise SystemExit(1)
PY
}

require_docker_ready() {
  command -v docker >/dev/null 2>&1 || {
    echo "docker CLI not found on PATH"
    return 1
  }
  python3 - "$DOCKER_INFO_TIMEOUT_SECONDS" <<'PY'
import subprocess
import sys

timeout = float(sys.argv[1])
try:
    result = subprocess.run(
        ["docker", "info"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        timeout=timeout,
    )
except subprocess.TimeoutExpired:
    print(f"docker info timed out after {timeout:g}s")
    raise SystemExit(1)

if result.returncode != 0:
    stderr = (result.stderr or "").strip().splitlines()
    detail = stderr[-1] if stderr else f"exit status {result.returncode}"
    print(f"docker info failed: {detail}")
    raise SystemExit(1)
PY
}

run_command_with_timeout() {
  local timeout_seconds="$1"
  shift
  python3 - "$timeout_seconds" "$@" <<'PY'
import subprocess
import sys

timeout = float(sys.argv[1])
cmd = sys.argv[2:]
try:
    raise SystemExit(subprocess.run(cmd, timeout=timeout).returncode)
except subprocess.TimeoutExpired:
    print(f"command timed out after {timeout:g}s: {' '.join(cmd)}", file=sys.stderr)
    raise SystemExit(124)
PY
}

run_step() {
  local name="$1"
  shift
  local step_dir="$OUT_DIR/$name"
  mkdir -p "$step_dir"
  echo "[remoteapp-cross-device-product-smoke] running $name"
  if run_command_with_timeout "$STEP_TIMEOUT_SECONDS" "$@" >"$step_dir/stdout.txt" 2>"$step_dir/stderr.txt"; then
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
  grep -q "distinct_device_uras_observed" "$0"
  grep -q "local_provider_boundary_only" "$0"
  grep -q "distinct device URAs were not observed" "$0"
  grep -q "local_provider_boundary_only=true" "$0"
  grep -q "requires_distinct_devices" "$0"
  grep -q "observed_device_pairs" "$0"
  grep -q "product_complete_claim" "$0"
  grep -q "service_owner_projection_failed" "$0"
  grep -q "real_os_window_application_capture" "$0"
  grep -q "does not prove real OS window/application capture" "$0"
  grep -q -- "--skip-build" "$0"
  grep -q "EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_MIN_FREE_KIB" "$0"
  grep -q "EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_DOCKER_INFO_TIMEOUT_SECONDS" "$0"
  grep -q "EASYNET_REMOTEAPP_CROSS_DEVICE_SMOKE_STEP_TIMEOUT_SECONDS" "$0"
  grep -q '"source"' "$0"
  grep -q '"runtime"' "$0"
  grep -q "image_created" "$0"
  grep -q "build_requested" "$0"
  grep -q "docker info timed out" "$0"
  grep -q "command timed out after" "$0"
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

if ! free_space_error="$(require_free_space 2>&1)"; then
  fail_preflight "$free_space_error"
fi

if ! docker_error="$(require_docker_ready 2>&1)"; then
  fail_preflight "$docker_error"
fi

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
if ! python3 - "$OUT_DIR/report.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    report = json.load(f)
if report.get("status") != "passed":
    print(report.get("reason") or "cross-device smoke failed", file=sys.stderr)
    raise SystemExit(1)
PY
then
  echo "[remoteapp-cross-device-product-smoke] FAIL: $OUT_DIR/report.md" >&2
  exit 1
fi
echo "[remoteapp-cross-device-product-smoke] PASS: $OUT_DIR/report.md"
