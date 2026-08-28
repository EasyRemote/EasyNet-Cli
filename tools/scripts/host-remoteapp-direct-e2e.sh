#!/usr/bin/env bash
# Reproducible host Browser + daemon RemoteApp direct-route proof.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
FRONTEND_ROOT="${EASYNET_REMOTEAPP_DIRECT_E2E_FRONTEND_ROOT:-$REPO_ROOT/../EasyNet/Frontend}"
PROJECTOR="$SELF_DIR/project-remoteapp-network-scenario.py"
VERIFIER="$SELF_DIR/remoteapp-network-fallback-e2e.sh"
RUNTIME_CLI="${EASYNET_REMOTEAPP_DIRECT_E2E_RUNTIME_CLI:-$REPO_ROOT/target/debug/easynet}"
BROWSER_RUNNER="$FRONTEND_ROOT/scripts/remoteapp-browser-lifecycle.mjs"
PROVIDER_STOP_ACTION="${EASYNET_REMOTEAPP_DIRECT_E2E_PROVIDER_STOP_ACTION:-}"
PROVIDER_START_ACTION="${EASYNET_REMOTEAPP_DIRECT_E2E_PROVIDER_START_ACTION:-}"
BROWSER_DOCKER="${EASYNET_REMOTEAPP_DIRECT_E2E_BROWSER_DOCKER:-0}"
BROWSER_IMAGE="${EASYNET_REMOTEAPP_DIRECT_E2E_BROWSER_IMAGE:-easynet/remoteapp-browser-chrome:e2e-152.0.7977.54}"
BROWSER_PLATFORM="${EASYNET_REMOTEAPP_DIRECT_E2E_BROWSER_PLATFORM:-linux/amd64}"
BROWSER_DOCKERFILE="$REPO_ROOT/tools/fixtures/remoteapp-browser-chrome/Dockerfile"
FRONTEND_PORT="${EASYNET_REMOTEAPP_DIRECT_E2E_FRONTEND_PORT:-3012}"

MODE=skip
OUT_DIR="${EASYNET_REMOTEAPP_DIRECT_E2E_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-direct/$(date -u +%Y%m%d-%H%M%S)-$$}"
FRONTEND_URL="${EASYNET_REMOTEAPP_DIRECT_E2E_FRONTEND_URL:-http://127.0.0.1:3000}"
DEVICE_ID="${EASYNET_REMOTEAPP_DIRECT_E2E_DEVICE_ID:-}"
TARGET_KIND="${EASYNET_REMOTEAPP_DIRECT_E2E_TARGET_KIND:-window}"

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-direct-e2e.sh --run --device-id UUID [--out-dir DIR]
  host-remoteapp-direct-e2e.sh --self-test

Required run environment:
  EASYNET_REMOTEAPP_BROWSER_EMAIL
  EASYNET_REMOTEAPP_BROWSER_PASSWORD

The runner restarts the paired development daemon with every RemoteApp
STUN/TURN/EasyNet relay variable removed, drives the real Browser RemoteApp
flow, and accepts only a host-only local/remote SDP and selected direct pair.
It restores the ordinary daemon afterward and emits only a focused direct
child proof, never a complete network fallback claim.

Set EASYNET_REMOTEAPP_DIRECT_E2E_BROWSER_DOCKER=1 to run Chromium on the
default Docker bridge for a direct host-candidate path to a container
provider. In that mode set both EASYNET_REMOTEAPP_DIRECT_E2E_PROVIDER_STOP_ACTION
and EASYNET_REMOTEAPP_DIRECT_E2E_PROVIDER_START_ACTION; the runner owns provider
and private frontend restoration.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
    --device-id) DEVICE_ID="${2:?missing value for --device-id}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

write_status() {
  local status="$1"
  local reason="$2"
  python3 - "$OUT_DIR/report.json" "$status" "$reason" <<'PY'
import json
import pathlib
import sys

path, status, reason = sys.argv[1:]
pathlib.Path(path).write_text(json.dumps({
    "script": "tools/scripts/host-remoteapp-direct-e2e.sh",
    "status": status,
    "reason": reason,
    "coverage": {"direct": status == "passed"},
    "product_complete_claim": False,
}, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

if [[ "$MODE" == "skip" ]]; then
  write_status skipped "pass --run to execute the live direct-route scenario"
  echo "[host-remoteapp-direct-e2e] skipped: $OUT_DIR/report.json"
  exit 0
fi

if [[ "$MODE" == "self-test" ]]; then
  bash -n "$0"
  grep -q 'CHROME_VERSION=152.0.7977.54' "$BROWSER_DOCKERFILE"
  grep -q '88af83664e1e5f79dc1c1378d0699b98dddd69690a748addf4ccbe322bfacedf' "$BROWSER_DOCKERFILE"
  python3 -m py_compile "$PROJECTOR"
  "$VERIFIER" --self-test --out-dir "$OUT_DIR/verifier-self-test" >/dev/null
  write_status passed "script syntax, projector import, and network evidence contract passed"
  echo "host-remoteapp-direct-e2e self-test ok"
  exit 0
fi

for command in node python3; do
  command -v "$command" >/dev/null 2>&1 || {
    write_status failed "required command is unavailable: $command"
    echo "required command is unavailable: $command" >&2
    exit 1
  }
done
EXTERNAL_PROVIDER=0
if [[ -n "$PROVIDER_STOP_ACTION" || -n "$PROVIDER_START_ACTION" ]]; then
  [[ -n "$PROVIDER_STOP_ACTION" && -n "$PROVIDER_START_ACTION" ]] || {
    write_status failed "external provider requires both stop and start actions"
    exit 64
  }
  for action in "$PROVIDER_STOP_ACTION" "$PROVIDER_START_ACTION"; do
    [[ "$action" != *$'\n'* && "$action" != *$'\r'* ]] || {
      write_status failed "provider lifecycle actions must be single-line shell commands"
      exit 64
    }
  done
  EXTERNAL_PROVIDER=1
else
  [[ -x "$RUNTIME_CLI" ]] || { write_status failed "runtime CLI missing: $RUNTIME_CLI"; exit 1; }
fi
[[ "$BROWSER_DOCKER" == 0 || "$BROWSER_DOCKER" == 1 ]] || {
  write_status failed "EASYNET_REMOTEAPP_DIRECT_E2E_BROWSER_DOCKER must be 0 or 1"
  exit 64
}
if [[ "$BROWSER_DOCKER" -eq 1 ]]; then
  for command in curl docker npm; do
    command -v "$command" >/dev/null 2>&1 || {
      write_status failed "required Docker Browser command is unavailable: $command"
      exit 1
    }
  done
  [[ "$EXTERNAL_PROVIDER" -eq 1 ]] || {
    write_status failed "Docker Browser direct proof requires an external provider lifecycle"
    exit 64
  }
fi
[[ -f "$BROWSER_RUNNER" ]] || { write_status failed "Browser runner missing: $BROWSER_RUNNER"; exit 1; }
[[ -f "$PROJECTOR" && -x "$VERIFIER" ]] || { write_status failed "network projector/verifier missing"; exit 1; }
[[ -n "$DEVICE_ID" ]] || { write_status failed "--device-id is required"; exit 64; }
: "${EASYNET_REMOTEAPP_BROWSER_EMAIL:?EASYNET_REMOTEAPP_BROWSER_EMAIL is required}"
: "${EASYNET_REMOTEAPP_BROWSER_PASSWORD:?EASYNET_REMOTEAPP_BROWSER_PASSWORD is required}"
[[ "$TARGET_KIND" == "window" || "$TARGET_KIND" == "application" ]] || {
  write_status failed "target kind must be window or application"
  exit 64
}

DAEMON_WAS_RUNNING=0
if [[ "$EXTERNAL_PROVIDER" -eq 0 ]]; then
  pgrep -f '/easynet-daemon$' >/dev/null 2>&1 && DAEMON_WAS_RUNNING=1
fi
RESTORE_FAILED=0
PROVIDER_MUTATED=0
FRONTEND_PID=""
BROWSER_CONTAINER="easynet-remoteapp-direct-browser-$$"

cleanup() {
  local exit_code=$?
  if [[ "$BROWSER_DOCKER" -eq 1 ]]; then
    docker rm -f "$BROWSER_CONTAINER" >/dev/null 2>&1 || true
  fi
  if [[ -n "$FRONTEND_PID" ]]; then
    kill "$FRONTEND_PID" >/dev/null 2>&1 || true
    wait "$FRONTEND_PID" >/dev/null 2>&1 || true
  fi
  if [[ "$EXTERNAL_PROVIDER" -eq 1 && "$PROVIDER_MUTATED" -eq 1 ]]; then
    /bin/sh -lc "$PROVIDER_STOP_ACTION" >/dev/null 2>&1 || true
    /bin/sh -lc "$PROVIDER_START_ACTION" >"$OUT_DIR/provider-restore.stdout.txt" \
      2>"$OUT_DIR/provider-restore.stderr.txt" || RESTORE_FAILED=1
  elif [[ "$EXTERNAL_PROVIDER" -eq 0 ]]; then
    "$RUNTIME_CLI" runtime stop >/dev/null 2>&1 || true
  fi
  if [[ "$EXTERNAL_PROVIDER" -eq 0 && "$DAEMON_WAS_RUNNING" -eq 1 ]]; then
    "$RUNTIME_CLI" runtime start >"$OUT_DIR/runtime-restore.stdout.txt" \
      2>"$OUT_DIR/runtime-restore.stderr.txt" || RESTORE_FAILED=1
  fi
  if [[ "$RESTORE_FAILED" -ne 0 && "$exit_code" -eq 0 ]]; then
    write_status failed "direct proof passed but standard local daemon restore failed"
    exit 1
  fi
  exit "$exit_code"
}
trap cleanup EXIT INT TERM

constraints_applied_at_ms="$(python3 -c 'import time; print(int(time.time() * 1000))')"
if [[ "$EXTERNAL_PROVIDER" -eq 1 ]]; then
  PROVIDER_MUTATED=1
  /bin/sh -lc "$PROVIDER_STOP_ACTION" >"$OUT_DIR/provider-stop.stdout.txt" \
    2>"$OUT_DIR/provider-stop.stderr.txt" || true
  env \
    -u EASYNET_REMOTE_DESKTOP_STUN_URLS \
    -u EASYNET_REMOTE_DESKTOP_TURN_URLS \
    -u EASYNET_REMOTE_DESKTOP_TURN_USERNAME \
    -u EASYNET_REMOTE_DESKTOP_TURN_CREDENTIAL \
    /bin/sh -lc "$PROVIDER_START_ACTION" >"$OUT_DIR/provider-start.stdout.txt" \
      2>"$OUT_DIR/provider-start.stderr.txt"
else
  "$RUNTIME_CLI" runtime stop >"$OUT_DIR/runtime-stop.stdout.txt" 2>"$OUT_DIR/runtime-stop.stderr.txt" || true
  env \
    -u EASYNET_REMOTE_DESKTOP_STUN_URLS \
    -u EASYNET_REMOTE_DESKTOP_TURN_URLS \
    -u EASYNET_REMOTE_DESKTOP_TURN_USERNAME \
    -u EASYNET_REMOTE_DESKTOP_TURN_CREDENTIAL \
    "$RUNTIME_CLI" runtime start >"$OUT_DIR/runtime-start.stdout.txt" 2>"$OUT_DIR/runtime-start.stderr.txt"
fi
sleep 2

browser_dir="$OUT_DIR/browser"
mkdir -p "$browser_dir"
if [[ "$BROWSER_DOCKER" -eq 1 ]]; then
  if ! docker image inspect "$BROWSER_IMAGE" >/dev/null 2>&1; then
    docker build --platform "$BROWSER_PLATFORM" --tag "$BROWSER_IMAGE" \
      --file "$BROWSER_DOCKERFILE" "$REPO_ROOT" \
      >"$OUT_DIR/browser-image-build.stdout.txt" 2>"$OUT_DIR/browser-image-build.stderr.txt"
  fi
  (
    cd "$FRONTEND_ROOT"
    __VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS=host.docker.internal \
    VITE_API_PROXY=http://127.0.0.1:8080 \
    npm run dev -- --host 0.0.0.0 --port "$FRONTEND_PORT" --strictPort
  ) >"$OUT_DIR/frontend.stdout.txt" 2>"$OUT_DIR/frontend.stderr.txt" &
  FRONTEND_PID=$!
  frontend_ready=0
  for _ in {1..200}; do
    if curl -fsS -H "Host: host.docker.internal:$FRONTEND_PORT" \
        "http://127.0.0.1:$FRONTEND_PORT/login" >/dev/null 2>&1; then
      frontend_ready=1
      break
    fi
    kill -0 "$FRONTEND_PID" >/dev/null 2>&1 || break
    sleep 0.1
  done
  [[ "$frontend_ready" -eq 1 ]] || { write_status failed "private frontend did not become ready"; exit 1; }
  docker run --rm --platform "$BROWSER_PLATFORM" --name "$BROWSER_CONTAINER" \
    --add-host host.docker.internal:host-gateway \
    --volume "$FRONTEND_ROOT:/workspace/frontend:ro" \
    --volume "$browser_dir:/out" \
    --workdir /workspace/frontend \
    --env EASYNET_REMOTEAPP_BROWSER_EMAIL \
    --env EASYNET_REMOTEAPP_BROWSER_PASSWORD \
    --env EASYNET_REMOTEAPP_BROWSER_DEVICE_ID="$DEVICE_ID" \
    --env EASYNET_REMOTEAPP_BROWSER_TARGET_KIND="$TARGET_KIND" \
    --env EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON=/out/evidence.json \
    --env EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL="http://host.docker.internal:$FRONTEND_PORT" \
    --env EASYNET_REMOTEAPP_BROWSER_INSECURE_ORIGIN_AS_SECURE="http://host.docker.internal:$FRONTEND_PORT" \
    --env EASYNET_REMOTEAPP_BROWSER_ICE_TRANSPORT_POLICY=all \
    "$BROWSER_IMAGE" \
    bash -lc 'chrome_path="${EASYNET_REMOTEAPP_BROWSER_CHROME_PATH:-$(find /ms-playwright -type f -path "*/chrome-linux/chrome" | head -n 1)}"; test -n "$chrome_path"; EASYNET_REMOTEAPP_BROWSER_CHROME_PATH="$chrome_path" node scripts/remoteapp-browser-lifecycle.mjs' \
    >"$browser_dir/runner.stdout.txt" 2>"$browser_dir/runner.stderr.txt"
else
  (
    cd "$FRONTEND_ROOT"
    EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON="$browser_dir/evidence.json" \
    EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL="$FRONTEND_URL" \
    EASYNET_REMOTEAPP_BROWSER_DEVICE_ID="$DEVICE_ID" \
    EASYNET_REMOTEAPP_BROWSER_TARGET_KIND="$TARGET_KIND" \
    EASYNET_REMOTEAPP_BROWSER_ICE_TRANSPORT_POLICY=all \
    node "$BROWSER_RUNNER"
  ) >"$browser_dir/runner.stdout.txt" 2>"$browser_dir/runner.stderr.txt"
fi

python3 "$PROJECTOR" \
  --browser-evidence "$browser_dir/evidence.json" \
  --route-kind direct \
  --constraints-applied-at-ms "$constraints_applied_at_ms" \
  --output "$OUT_DIR/evidence.json"

"$VERIFIER" --run --required-routes direct \
  --evidence-json "$OUT_DIR/evidence.json" --out-dir "$OUT_DIR/verified" >/dev/null
cp "$OUT_DIR/verified/report.json" "$OUT_DIR/report.json"
echo "[host-remoteapp-direct-e2e] PASS: $OUT_DIR/report.json"
