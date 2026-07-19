#!/usr/bin/env bash
# Host-gated E2E for real local media hardware paths.
#
# This script intentionally does NOT run by default. Real mic/camera/screen
# tests may trigger OS permission prompts and depend on an interactive user
# session. Enable explicitly with `--run` or EASYNET_HOST_MEDIA_E2E=1.
#
# Scope:
#   - mic.subscribe through `easynet ability record`
#   - camera.record_start/camera.record_stop through `easynet ability record`
#   - screen.snapshot through `easynet ability invoke`
#
# What this is not:
#   - It is not the Docker synthetic stream/bidi gate.
#   - It is not a WebRTC media-quality test.
#   - It is not allowed to call daemon internals.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="${EASYNET_HOST_MEDIA_E2E_OUT_DIR:-$REPO_ROOT/target/e2e/host-media-device/$TIMESTAMP}"
RUN="${EASYNET_HOST_MEDIA_E2E:-0}"
ALLOW_START=1
SELF_TEST=0
MAX_MIC_FRAMES="${EASYNET_HOST_MEDIA_MIC_FRAMES:-8}"
CAMERA_DURATION_MS="${EASYNET_HOST_MEDIA_CAMERA_DURATION_MS:-1500}"
TIMEOUT_SECS="${EASYNET_HOST_MEDIA_TIMEOUT_SECS:-30}"
REQUIRED_KINDS=()

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/host-media-device-e2e.sh [options]

Options:
  --run                 Actually run host hardware checks.
  --no-start            Do not start the local daemon when it is stopped.
  --require KIND        Fail if KIND is unavailable or capture fails.
                        KIND must be one of: mic, camera, screen. Repeatable.
  --out-dir DIR         Report directory.
  --mic-frames N        Number of mic frames to record. Default: 8.
  --camera-duration-ms N
                        Camera recording duration. Default: 1500.
  --timeout SECS        Per-command timeout. Default: 30.
  --self-test           Validate script structure without touching hardware.
  -h, --help            Show this help.

Environment:
  EASYNET_HOST_MEDIA_E2E=1
                        Equivalent to --run.
  EASYNET_BIN           EasyNet CLI binary. Default: easynet.

Exit semantics:
  0  all required checks passed, or script was skipped because not enabled.
  1  a required host media check failed.

Examples:
  EASYNET_HOST_MEDIA_E2E=1 tools/scripts/host-media-device-e2e.sh
  tools/scripts/host-media-device-e2e.sh --run --require mic --require camera
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) RUN=1; shift ;;
    --no-start) ALLOW_START=0; shift ;;
    --require)
      case "${2:?missing value for --require}" in
        mic|camera|screen) REQUIRED_KINDS+=("$2") ;;
        *) echo "invalid required kind: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --mic-frames) MAX_MIC_FRAMES="${2:?missing value for --mic-frames}"; shift 2 ;;
    --camera-duration-ms) CAMERA_DURATION_MS="${2:?missing value for --camera-duration-ms}"; shift 2 ;;
    --timeout) TIMEOUT_SECS="${2:?missing value for --timeout}"; shift 2 ;;
    --self-test) SELF_TEST=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

EASYNET_BIN="${EASYNET_BIN:-easynet}"

die() {
  echo "[FAIL] $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

is_required() {
  local kind="$1"
  local required
  for required in "${REQUIRED_KINDS[@]}"; do
    [[ "$required" == "$kind" ]] && return 0
  done
  return 1
}

random_nonce_hex() {
  openssl rand -hex 16
}

json_quote() {
  python3 - "$1" <<'PY'
import json
import sys
print(json.dumps(sys.argv[1]))
PY
}

if [[ "$SELF_TEST" == "1" ]]; then
  bash -n "$0"
  grep -q "EASYNET_HOST_MEDIA_E2E" "$0"
  grep -q "ability record" "$0"
  grep -q "camera.record_start" "$0"
  grep -q "screen.snapshot" "$0"
  grep -q "meta.list_resources" "$0"
  grep -q "random_nonce_hex" "$0"
  grep -q "hardware" "$0"
  echo "host-media-device-e2e self-test ok"
  exit 0
fi

mkdir -p "$OUT_DIR"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"

if [[ "$RUN" != "1" ]]; then
  python3 - "$REPORT_JSON" <<'PY'
import json
import sys
report = {
    "enabled": False,
    "status": "skipped",
    "reason": "host media e2e requires --run or EASYNET_HOST_MEDIA_E2E=1",
    "assertions": {},
}
open(sys.argv[1], "w", encoding="utf-8").write(json.dumps(report, indent=2, sort_keys=True) + "\n")
PY
  {
    echo "# Host media device E2E report"
    echo
    echo "- Status: \`skipped\`"
    echo "- Reason: \`host media e2e requires --run or EASYNET_HOST_MEDIA_E2E=1\`"
  } >"$REPORT_MD"
  echo "SKIP: $REPORT_MD"
  exit 0
fi

need_cmd "$EASYNET_BIN"
need_cmd jq
need_cmd openssl
need_cmd python3

run_cli() {
  "$EASYNET_BIN" "$@"
}

echo "==> checking local daemon"
set +e
run_cli runtime status --json >"$OUT_DIR/runtime-status-before.json" 2>"$OUT_DIR/runtime-status-before.err"
STATUS_RC=$?
set -e
if [[ "$STATUS_RC" -ne 0 ]] || ! jq -e '(.runtime_status // "") != "stopped"' "$OUT_DIR/runtime-status-before.json" >/dev/null 2>&1; then
  if [[ "$ALLOW_START" != "1" ]]; then
    die "local runtime is not running and --no-start was supplied"
  fi
  run_cli runtime start >"$OUT_DIR/runtime-start.txt" 2>"$OUT_DIR/runtime-start.err"
fi
run_cli runtime status --json >"$OUT_DIR/runtime-status.json" 2>"$OUT_DIR/runtime-status.err"

echo "==> listing local abilities"
run_cli ability list --format json >"$OUT_DIR/ability-list.json" 2>"$OUT_DIR/ability-list.err"

python3 - "$OUT_DIR/ability-list.json" "$OUT_DIR/ability-map.json" <<'PY'
import json
import sys

payload = json.load(open(sys.argv[1], encoding="utf-8"))
rows = payload if isinstance(payload, list) else payload.get("abilities") or payload.get("items") or payload.get("records") or []
wanted = {
    "meta.list_resources",
    "mic.subscribe",
    "camera.subscribe",
    "camera.record_start",
    "screen.snapshot",
}
out = {}
for row in rows:
    if not isinstance(row, dict):
        continue
    names = {
        str(row.get("name") or ""),
        str(row.get("ability_name") or ""),
        str(row.get("public_name") or ""),
        str(row.get("qualified_name") or ""),
    }
    ability_ura = str(row.get("ability_ura") or "")
    for name in wanted:
        if name in names or ability_ura.endswith("." + name):
            out[name] = ability_ura
json.dump(out, open(sys.argv[2], "w", encoding="utf-8"), indent=2, sort_keys=True)
PY

ability_ura() {
  local name="$1"
  jq -r --arg name "$name" '.[$name] // ""' "$OUT_DIR/ability-map.json"
}

META_URA="$(ability_ura meta.list_resources)"
MIC_URA="$(ability_ura mic.subscribe)"
CAMERA_RECORD_URA="$(ability_ura camera.record_start)"
CAMERA_SUBSCRIBE_URA="$(ability_ura camera.subscribe)"
SCREEN_SNAPSHOT_URA="$(ability_ura screen.snapshot)"
[[ -n "$META_URA" ]] || die "meta.list_resources ability is not available"

REALM="$(python3 - "$META_URA" <<'PY'
import re
import sys
m = re.search(r"easynet:///r/([^/]+)/", sys.argv[1])
print(m.group(1) if m else "localhost")
PY
)"
LIST_SUBJECT="easynet:///r/${REALM}/resource/e2e/host-media/list-resources"

echo "==> listing real local media resources"
RESOURCE_TYPES_JSON='{"types":["mic","camera","display","window","application"]}'
run_cli ability invoke "$META_URA" \
  --subject "$LIST_SUBJECT" \
  --nonce-hex "$(random_nonce_hex)" \
  --causal-root \
  --args "$RESOURCE_TYPES_JSON" \
  --raw \
  >"$OUT_DIR/resources.json" 2>"$OUT_DIR/resources.err"

python3 - "$OUT_DIR/resources.json" "$OUT_DIR/resource-map.json" <<'PY'
import json
import sys

text = open(sys.argv[1], encoding="utf-8").read()
start = text.find("{")
end = text.rfind("}")
payload = json.loads(text[start:end + 1]) if start >= 0 and end >= start else {}
resources = payload.get("resources") or []
out = {"mic": "", "camera": "", "screen": ""}
for row in resources:
    if not isinstance(row, dict):
        continue
    kind = str(row.get("type") or "")
    resource_ura = str(row.get("resource_ura") or "").strip()
    if not resource_ura:
        continue
    if kind == "mic" and not out["mic"]:
        out["mic"] = resource_ura
    if kind == "camera" and not out["camera"]:
        out["camera"] = resource_ura
    if kind in {"display", "window", "application"} and not out["screen"]:
        out["screen"] = resource_ura
json.dump(out, open(sys.argv[2], "w", encoding="utf-8"), indent=2, sort_keys=True)
PY

resource_ura() {
  local kind="$1"
  jq -r --arg kind "$kind" '.[$kind] // ""' "$OUT_DIR/resource-map.json"
}

MIC_RESOURCE="$(resource_ura mic)"
CAMERA_RESOURCE="$(resource_ura camera)"
SCREEN_RESOURCE="$(resource_ura screen)"

declare -A RESULT_STATUS
declare -A RESULT_REASON
declare -A RESULT_REQUIRED

record_result() {
  local kind="$1"
  local status="$2"
  local reason="$3"
  RESULT_STATUS["$kind"]="$status"
  RESULT_REASON["$kind"]="$reason"
  if is_required "$kind"; then
    RESULT_REQUIRED["$kind"]="true"
  else
    RESULT_REQUIRED["$kind"]="false"
  fi
}

run_optional_check() {
  local kind="$1"
  shift
  set +e
  "$@"
  local rc=$?
  set -e
  return "$rc"
}

echo "==> checking microphone capture"
if [[ -z "$MIC_URA" ]]; then
  record_result mic skipped "mic.subscribe ability is unavailable"
elif [[ -z "$MIC_RESOURCE" ]]; then
  record_result mic skipped "no mic resource was advertised by meta.list_resources"
else
  MIC_OUT="$OUT_DIR/mic-recording"
  if run_optional_check mic run_cli ability record "$MIC_URA" \
    --subject "$MIC_RESOURCE" \
    --max-frames "$MAX_MIC_FRAMES" \
    --timeout "$TIMEOUT_SECS" \
    --output-dir "$MIC_OUT" \
    >"$OUT_DIR/mic-record.txt" 2>"$OUT_DIR/mic-record.err"; then
    if find "$MIC_OUT" -name manifest.json -type f -print -quit | grep -q .; then
      record_result mic passed "mic recording artifact manifest produced"
    else
      record_result mic failed "mic record command succeeded but no artifact manifest was produced"
    fi
  else
    record_result mic failed "mic record command failed"
  fi
fi

echo "==> checking camera recording"
if [[ -z "$CAMERA_RECORD_URA" && -z "$CAMERA_SUBSCRIBE_URA" ]]; then
  record_result camera skipped "camera recording ability is unavailable"
elif [[ -z "$CAMERA_RESOURCE" ]]; then
  record_result camera skipped "no camera resource was advertised by meta.list_resources"
else
  CAMERA_URA="${CAMERA_RECORD_URA:-$CAMERA_SUBSCRIBE_URA}"
  if run_optional_check camera run_cli ability record "$CAMERA_URA" \
    --subject "$CAMERA_RESOURCE" \
    --duration-ms "$CAMERA_DURATION_MS" \
    --timeout "$TIMEOUT_SECS" \
    --print-frames \
    >"$OUT_DIR/camera-record.txt" 2>"$OUT_DIR/camera-record.err"; then
    record_result camera passed "camera recording command completed"
  else
    record_result camera failed "camera recording command failed"
  fi
fi

echo "==> checking screen snapshot"
if [[ -z "$SCREEN_SNAPSHOT_URA" ]]; then
  record_result screen skipped "screen.snapshot ability is unavailable"
elif [[ -z "$SCREEN_RESOURCE" ]]; then
  record_result screen skipped "no display/window/application resource was advertised by meta.list_resources"
else
  if run_optional_check screen run_cli ability invoke "$SCREEN_SNAPSHOT_URA" \
    --subject "$SCREEN_RESOURCE" \
    --nonce-hex "$(random_nonce_hex)" \
    --causal-root \
    --args '{}' \
    --raw \
    >"$OUT_DIR/screen-snapshot.json" 2>"$OUT_DIR/screen-snapshot.err"; then
    if python3 - "$OUT_DIR/screen-snapshot.json" <<'PY'
import json
import sys
text = open(sys.argv[1], encoding="utf-8").read()
start = text.find("{")
end = text.rfind("}")
payload = json.loads(text[start:end + 1]) if start >= 0 and end >= start else {}
blob = json.dumps(payload)
raise SystemExit(0 if ("image_bytes_b64" in blob or "payloadstore_ura" in blob or "local_path" in blob) else 1)
PY
    then
      record_result screen passed "screen snapshot produced image payload metadata"
    else
      record_result screen failed "screen snapshot command succeeded but no image payload metadata was found"
    fi
  else
    record_result screen failed "screen snapshot command failed"
  fi
fi

python3 - "$REPORT_JSON" "$OUT_DIR" "$META_URA" "$MIC_URA" "$CAMERA_RECORD_URA" "$SCREEN_SNAPSHOT_URA" \
  "$(json_quote "${RESULT_STATUS[mic]:-unknown}")" "$(json_quote "${RESULT_REASON[mic]:-not run}")" "$(json_quote "${RESULT_REQUIRED[mic]:-false}")" \
  "$(json_quote "${RESULT_STATUS[camera]:-unknown}")" "$(json_quote "${RESULT_REASON[camera]:-not run}")" "$(json_quote "${RESULT_REQUIRED[camera]:-false}")" \
  "$(json_quote "${RESULT_STATUS[screen]:-unknown}")" "$(json_quote "${RESULT_REASON[screen]:-not run}")" "$(json_quote "${RESULT_REQUIRED[screen]:-false}")" <<'PY'
import json
import pathlib
import sys

(
    report_path,
    out_dir,
    meta_ura,
    mic_ura,
    camera_record_ura,
    screen_snapshot_ura,
    mic_status,
    mic_reason,
    mic_required,
    camera_status,
    camera_reason,
    camera_required,
    screen_status,
    screen_reason,
    screen_required,
) = sys.argv[1:16]

def decoded(value):
    return json.loads(value)

checks = {
    "mic": {
        "status": decoded(mic_status),
        "reason": decoded(mic_reason),
        "required": decoded(mic_required) == "true",
    },
    "camera": {
        "status": decoded(camera_status),
        "reason": decoded(camera_reason),
        "required": decoded(camera_required) == "true",
    },
    "screen": {
        "status": decoded(screen_status),
        "reason": decoded(screen_reason),
        "required": decoded(screen_required) == "true",
    },
}
assertions = {}
for kind, check in checks.items():
    if check["required"]:
        assertions[f"{kind}_required_passed"] = check["status"] == "passed"
    elif check["status"] == "failed":
        assertions[f"{kind}_optional_did_not_fail"] = False
    else:
        assertions[f"{kind}_optional_completed_or_skipped"] = True

report = {
    "enabled": True,
    "status": "failed" if any(value is False for value in assertions.values()) else "passed",
    "out_dir": out_dir,
    "abilities": {
        "meta_list_resources": meta_ura,
        "mic_subscribe": mic_ura,
        "camera_record_start": camera_record_ura,
        "screen_snapshot": screen_snapshot_ura,
    },
    "checks": checks,
    "assertions": assertions,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

python3 - "$REPORT_JSON" "$REPORT_MD" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
with open(sys.argv[2], "w", encoding="utf-8") as f:
    f.write("# Host media device E2E report\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Output dir: `{report['out_dir']}`\n\n")
    f.write("## Checks\n\n")
    for kind, check in report["checks"].items():
        f.write(f"- `{kind}`: `{check['status']}` — {check['reason']} (required={str(check['required']).lower()})\n")
    f.write("\n## Assertions\n\n")
    for key, value in report["assertions"].items():
        f.write(f"- `{key}`: `{str(value).lower()}`\n")
PY

if jq -e '.status == "passed"' "$REPORT_JSON" >/dev/null; then
  echo "PASS: $REPORT_MD"
else
  echo "FAIL: $REPORT_MD" >&2
  exit 1
fi
