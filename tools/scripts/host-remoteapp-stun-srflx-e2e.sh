#!/usr/bin/env bash
# Reproducible host Browser + daemon + VM-NAT RemoteApp STUN proof.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
FRONTEND_ROOT="${EASYNET_REMOTEAPP_STUN_E2E_FRONTEND_ROOT:-$REPO_ROOT/../EasyNet/Frontend}"
PROJECTOR="$SELF_DIR/project-remoteapp-network-scenario.py"
VERIFIER="$SELF_DIR/remoteapp-network-fallback-e2e.sh"
STUN_SERVER="$SELF_DIR/remoteapp-stun-binding-server.py"
RUNTIME_CLI="${EASYNET_REMOTEAPP_STUN_E2E_RUNTIME_CLI:-$REPO_ROOT/target/debug/easynet}"
BROWSER_RUNNER="$FRONTEND_ROOT/scripts/remoteapp-browser-lifecycle.mjs"
PROVIDER_STOP_ACTION="${EASYNET_REMOTEAPP_STUN_E2E_PROVIDER_STOP_ACTION:-}"
PROVIDER_START_ACTION="${EASYNET_REMOTEAPP_STUN_E2E_PROVIDER_START_ACTION:-}"

MODE=skip
OUT_DIR="${EASYNET_REMOTEAPP_STUN_E2E_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-stun-srflx/$(date -u +%Y%m%d-%H%M%S)-$$}"
DEVICE_ID="${EASYNET_REMOTEAPP_STUN_E2E_DEVICE_ID:-}"
TARGET_KIND="${EASYNET_REMOTEAPP_STUN_E2E_TARGET_KIND:-window}"
STUN_PORT="${EASYNET_REMOTEAPP_STUN_E2E_PORT:-3482}"
STUN_HOST="${EASYNET_REMOTEAPP_STUN_E2E_HOST:-}"
FRONTEND_PORT="${EASYNET_REMOTEAPP_STUN_E2E_FRONTEND_PORT:-3011}"
BROWSER_IMAGE="${EASYNET_REMOTEAPP_STUN_E2E_BROWSER_IMAGE:-easynet/remoteapp-browser-chrome:e2e-152.0.7977.54}"
BROWSER_PLATFORM="${EASYNET_REMOTEAPP_STUN_E2E_BROWSER_PLATFORM:-linux/amd64}"
BROWSER_DOCKERFILE="$REPO_ROOT/tools/fixtures/remoteapp-browser-chrome/Dockerfile"
BROWSER_RUN_DEADLINE_SECONDS="${EASYNET_REMOTEAPP_STUN_E2E_BROWSER_RUN_DEADLINE_SECONDS:-240}"
BROWSER_DOCKER_CONTEXT="${EASYNET_REMOTEAPP_STUN_E2E_BROWSER_DOCKER_CONTEXT:-}"

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-stun-srflx-e2e.sh --run --device-id UUID [--out-dir DIR]
  host-remoteapp-stun-srflx-e2e.sh --self-test

Required run environment:
  EASYNET_REMOTEAPP_BROWSER_EMAIL
  EASYNET_REMOTEAPP_BROWSER_PASSWORD

The runner starts a bounded RFC 5389 Binding-only fixture on the provider host,
configures the paired daemon with only its STUN URL, serves the real frontend
on a private E2E port, and drives Chromium from an externally reachable VM-NAT
Docker context. An E2E-only native peer constraint admits only srflx/prflx
Browser outbound candidates while retaining host/srflx/prflx provider inbound
return candidates. A server binding, selected pair, later media and terminal
receipt are all required. The ordinary daemon is restored afterward.

Environment:
  EASYNET_REMOTEAPP_STUN_E2E_BROWSER_DOCKER_CONTEXT
                Docker context for an externally reachable VM-NAT peer. On
                macOS, Docker Desktop is rejected because its hidden VM NAT
                does not expose a return route for reflexive-only ICE.
  EASYNET_REMOTEAPP_STUN_E2E_PROVIDER_STOP_ACTION
  EASYNET_REMOTEAPP_STUN_E2E_PROVIDER_START_ACTION
                Single-line lifecycle actions for the external provider. The
                start action receives EASYNET_REMOTE_DESKTOP_STUN_URLS and must
                explicitly pass it into the provider process.
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
    "script": "tools/scripts/host-remoteapp-stun-srflx-e2e.sh",
    "status": status,
    "reason": reason,
    "coverage": {"stun_srflx": status == "passed"},
    "product_complete_claim": False,
}, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

verify_browser_candidate_boundary() {
  [[ -f "$BROWSER_RUNNER" ]] || {
    write_status failed "Browser lifecycle runner missing: $BROWSER_RUNNER"
    return 1
  }
  node --check "$BROWSER_RUNNER" >/dev/null || {
    write_status failed "Browser lifecycle runner has invalid JavaScript syntax"
    return 1
  }
  grep -Fq "get localDescription()" "$BROWSER_RUNNER" || {
    write_status failed "Browser lifecycle runner does not intercept embedded local SDP candidates"
    return 1
  }
  grep -Fq "admittedDescription('outbound', super.localDescription)" "$BROWSER_RUNNER" || {
    write_status failed "Browser lifecycle runner does not apply outbound admission to local SDP"
    return 1
  }
}

if [[ "$MODE" == "skip" ]]; then
  write_status skipped "pass --run to execute the live STUN srflx scenario"
  echo "[host-remoteapp-stun-srflx-e2e] skipped: $OUT_DIR/report.json"
  exit 0
fi

if [[ "$MODE" == "self-test" ]]; then
  bash -n "$0"
  grep -q 'CHROME_VERSION=152.0.7977.54' "$BROWSER_DOCKERFILE"
  grep -q '88af83664e1e5f79dc1c1378d0699b98dddd69690a748addf4ccbe322bfacedf' "$BROWSER_DOCKERFILE"
  python3 -m py_compile "$PROJECTOR"
  python3 -m py_compile "$STUN_SERVER"
  verify_browser_candidate_boundary
  "$VERIFIER" --self-test --out-dir "$OUT_DIR/verifier-self-test" >/dev/null
  write_status passed "script syntax, projector import, and network evidence contract passed"
  echo "host-remoteapp-stun-srflx-e2e self-test ok"
  exit 0
fi

for command in curl docker node npm python3; do
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
verify_browser_candidate_boundary
[[ -f "$PROJECTOR" && -x "$VERIFIER" && -x "$STUN_SERVER" ]] || {
  write_status failed "network projector/verifier or native STUN fixture missing"
  exit 1
}
[[ -n "$DEVICE_ID" ]] || { write_status failed "--device-id is required"; exit 64; }
: "${EASYNET_REMOTEAPP_BROWSER_EMAIL:?EASYNET_REMOTEAPP_BROWSER_EMAIL is required}"
: "${EASYNET_REMOTEAPP_BROWSER_PASSWORD:?EASYNET_REMOTEAPP_BROWSER_PASSWORD is required}"
[[ "$TARGET_KIND" == "window" || "$TARGET_KIND" == "application" ]] || {
  write_status failed "target kind must be window or application"
  exit 64
}
[[ "$STUN_PORT" =~ ^[0-9]+$ ]] && (( STUN_PORT > 0 && STUN_PORT <= 65535 )) || {
  write_status failed "STUN port must be an integer in 1..65535"
  exit 64
}
[[ "$FRONTEND_PORT" =~ ^[0-9]+$ ]] && (( FRONTEND_PORT > 0 && FRONTEND_PORT <= 65535 )) || {
  write_status failed "frontend port must be an integer in 1..65535"
  exit 64
}
[[ "$BROWSER_RUN_DEADLINE_SECONDS" =~ ^[0-9]+$ ]] && (( BROWSER_RUN_DEADLINE_SECONDS > 0 )) || {
  write_status failed "Browser run deadline must be a positive integer number of seconds"
  exit 64
}
if [[ -z "$STUN_HOST" ]]; then
  STUN_HOST="$(ipconfig getifaddr en0 2>/dev/null || true)"
fi
[[ -n "$STUN_HOST" && "$STUN_HOST" != "127.0.0.1" && "$STUN_HOST" != "localhost" ]] || {
  write_status failed "a non-loopback STUN host is required; set EASYNET_REMOTEAPP_STUN_E2E_HOST"
  exit 64
}
if [[ -z "$BROWSER_DOCKER_CONTEXT" ]]; then
  BROWSER_DOCKER_CONTEXT="$(docker context show 2>/dev/null || true)"
fi
[[ -n "$BROWSER_DOCKER_CONTEXT" ]] || {
  write_status failed "a Browser Docker context is required"
  exit 64
}
if [[ "$(uname -s)" == "Darwin" && "$BROWSER_DOCKER_CONTEXT" == "desktop-linux" ]]; then
  write_status failed \
    "macOS STUN proof requires an externally reachable VM-NAT Docker context; Docker Desktop has no reflexive ICE return route"
  exit 64
fi
if ! docker --context "$BROWSER_DOCKER_CONTEXT" info >/dev/null 2>&1; then
  write_status failed "Browser Docker context is not reachable: $BROWSER_DOCKER_CONTEXT"
  exit 1
fi

TEMP_DIR="$(mktemp -d)"
BROWSER_CONTAINER="easynet-remoteapp-stun-browser-$$"
DAEMON_WAS_RUNNING=0
if [[ "$EXTERNAL_PROVIDER" -eq 0 ]]; then
  pgrep -f '/easynet-daemon$' >/dev/null 2>&1 && DAEMON_WAS_RUNNING=1
fi
RESTORE_FAILED=0
PROVIDER_MUTATED=0
FRONTEND_PID=""
BROWSER_DOCKER_PID=""
STUN_SERVER_PID=""

browser_docker() {
  docker --context "$BROWSER_DOCKER_CONTEXT" "$@"
}

bounded_stop_container() {
  local container_name="$1"
  python3 - "$BROWSER_DOCKER_CONTEXT" "$container_name" <<'PY'
import subprocess
import sys

context, container = sys.argv[1:]
for command in (["docker", "stop", container], ["docker", "rm", "-f", container]):
    try:
        completed = subprocess.run(
            ["docker", "--context", context, *command[1:]],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=10,
            check=False,
        )
    except subprocess.TimeoutExpired:
        continue
    if completed.returncode == 0:
        break
PY
}

cleanup() {
  local exit_code=$?
  if [[ "$EXTERNAL_PROVIDER" -eq 0 ]]; then
    "$RUNTIME_CLI" runtime stop >/dev/null 2>&1 || true
  fi
  if [[ -n "$BROWSER_DOCKER_PID" ]] && kill -0 "$BROWSER_DOCKER_PID" >/dev/null 2>&1; then
    kill "$BROWSER_DOCKER_PID" >/dev/null 2>&1 || true
    wait "$BROWSER_DOCKER_PID" >/dev/null 2>&1 || true
  fi
  bounded_stop_container "$BROWSER_CONTAINER"
  if [[ -n "$STUN_SERVER_PID" ]] && kill -0 "$STUN_SERVER_PID" >/dev/null 2>&1; then
    kill -TERM "$STUN_SERVER_PID" >/dev/null 2>&1 || true
    wait "$STUN_SERVER_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$FRONTEND_PID" ]]; then
    kill "$FRONTEND_PID" >/dev/null 2>&1 || true
    wait "$FRONTEND_PID" >/dev/null 2>&1 || true
  fi
  if [[ "$EXTERNAL_PROVIDER" -eq 1 && "$PROVIDER_MUTATED" -eq 1 ]]; then
    /bin/sh -lc "$PROVIDER_STOP_ACTION" >/dev/null 2>&1 || true
    EASYNET_REMOTE_DESKTOP_STUN_URLS= \
      /bin/sh -lc "$PROVIDER_START_ACTION" >"$OUT_DIR/provider-restore.stdout.txt" \
      2>"$OUT_DIR/provider-restore.stderr.txt" || RESTORE_FAILED=1
  elif [[ "$DAEMON_WAS_RUNNING" -eq 1 ]]; then
    "$RUNTIME_CLI" runtime start >"$OUT_DIR/runtime-restore.stdout.txt" \
      2>"$OUT_DIR/runtime-restore.stderr.txt" || RESTORE_FAILED=1
  fi
  rm -rf "$TEMP_DIR"
  if [[ "$RESTORE_FAILED" -ne 0 && "$exit_code" -eq 0 ]]; then
    write_status failed "STUN proof passed but standard local daemon restore failed"
    exit 1
  fi
  exit "$exit_code"
}
trap cleanup EXIT INT TERM

if ! browser_docker image inspect "$BROWSER_IMAGE" >/dev/null 2>&1; then
  browser_docker build --platform "$BROWSER_PLATFORM" --tag "$BROWSER_IMAGE" \
    --file "$BROWSER_DOCKERFILE" "$REPO_ROOT" \
    >"$OUT_DIR/browser-image-build.stdout.txt" 2>"$OUT_DIR/browser-image-build.stderr.txt"
fi
STUN_EVENT_LOG="$OUT_DIR/stun-binding-events.jsonl"
STUN_READY_FILE="$TEMP_DIR/stun-ready.json"
"$STUN_SERVER" \
  --listen-host "$STUN_HOST" \
  --listen-port "$STUN_PORT" \
  --event-log "$STUN_EVENT_LOG" \
  --ready-file "$STUN_READY_FILE" \
  >"$OUT_DIR/stun-server.stdout.txt" 2>"$OUT_DIR/stun-server.stderr.txt" &
STUN_SERVER_PID=$!

ready=0
for _ in {1..100}; do
  if [[ -s "$STUN_READY_FILE" ]]; then
    ready=1
    break
  fi
  if ! kill -0 "$STUN_SERVER_PID" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done
[[ "$ready" -eq 1 ]] || { write_status failed "native STUN fixture did not become ready"; exit 1; }

constraints_applied_at_ms="$(python3 -c 'import time; print(int(time.time() * 1000))')"
if [[ "$EXTERNAL_PROVIDER" -eq 1 ]]; then
  PROVIDER_MUTATED=1
  /bin/sh -lc "$PROVIDER_STOP_ACTION" >"$OUT_DIR/provider-stop.stdout.txt" \
    2>"$OUT_DIR/provider-stop.stderr.txt" || true
  env \
    -u EASYNET_REMOTE_DESKTOP_TURN_URLS \
    -u EASYNET_REMOTE_DESKTOP_TURN_USERNAME \
    -u EASYNET_REMOTE_DESKTOP_TURN_CREDENTIAL \
    EASYNET_REMOTE_DESKTOP_STUN_URLS="stun:$STUN_HOST:$STUN_PORT" \
    /bin/sh -lc "$PROVIDER_START_ACTION" >"$OUT_DIR/provider-start.stdout.txt" \
      2>"$OUT_DIR/provider-start.stderr.txt"
else
  "$RUNTIME_CLI" runtime stop >"$OUT_DIR/runtime-stop.stdout.txt" 2>"$OUT_DIR/runtime-stop.stderr.txt" || true
  env \
    -u EASYNET_REMOTE_DESKTOP_TURN_URLS \
    -u EASYNET_REMOTE_DESKTOP_TURN_USERNAME \
    -u EASYNET_REMOTE_DESKTOP_TURN_CREDENTIAL \
    EASYNET_REMOTE_DESKTOP_STUN_URLS="stun:$STUN_HOST:$STUN_PORT" \
    "$RUNTIME_CLI" runtime start >"$OUT_DIR/runtime-start.stdout.txt" 2>"$OUT_DIR/runtime-start.stderr.txt"
fi
sleep 2

(
  cd "$FRONTEND_ROOT"
  __VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS="$STUN_HOST" \
  VITE_API_PROXY=http://127.0.0.1:8080 \
  npm run dev -- --host 0.0.0.0 --port "$FRONTEND_PORT" --strictPort
) >"$OUT_DIR/frontend.stdout.txt" 2>"$OUT_DIR/frontend.stderr.txt" &
FRONTEND_PID=$!
frontend_ready=0
for _ in {1..200}; do
  if ! kill -0 "$FRONTEND_PID" >/dev/null 2>&1; then
    write_status failed "private frontend exited before readiness"
    exit 1
  fi
  if curl -fsS -H "Host: $STUN_HOST:$FRONTEND_PORT" \
      "http://127.0.0.1:$FRONTEND_PORT/login" >/dev/null 2>&1; then
    frontend_ready=1
    break
  fi
  sleep 0.1
done
[[ "$frontend_ready" -eq 1 ]] || { write_status failed "private frontend did not become ready"; exit 1; }

browser_dir="$OUT_DIR/browser"
mkdir -p "$browser_dir"
EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON=/out/evidence.json \
EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL="http://$STUN_HOST:$FRONTEND_PORT" \
EASYNET_REMOTEAPP_BROWSER_DEVICE_ID="$DEVICE_ID" \
EASYNET_REMOTEAPP_BROWSER_TARGET_KIND="$TARGET_KIND" \
EASYNET_REMOTEAPP_BROWSER_ICE_TRANSPORT_POLICY=all \
EASYNET_REMOTEAPP_BROWSER_ALLOWED_OUTBOUND_ICE_CANDIDATE_TYPES=srflx,prflx \
EASYNET_REMOTEAPP_BROWSER_ALLOWED_INBOUND_ICE_CANDIDATE_TYPES=host,srflx,prflx \
browser_docker run --rm --name "$BROWSER_CONTAINER" \
  --platform "$BROWSER_PLATFORM" \
  --add-host host.docker.internal:host-gateway \
  --volume "$FRONTEND_ROOT:/workspace/frontend:ro" \
  --volume "$browser_dir:/out" \
  --workdir /workspace/frontend \
  --env EASYNET_REMOTEAPP_BROWSER_EMAIL \
  --env EASYNET_REMOTEAPP_BROWSER_PASSWORD \
  --env EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON \
  --env EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL \
  --env EASYNET_REMOTEAPP_BROWSER_INSECURE_ORIGIN_AS_SECURE="http://$STUN_HOST:$FRONTEND_PORT" \
  --env EASYNET_REMOTEAPP_BROWSER_DISABLE_GPU=1 \
  --env EASYNET_REMOTEAPP_BROWSER_DEVICE_ID \
  --env EASYNET_REMOTEAPP_BROWSER_TARGET_KIND \
  --env EASYNET_REMOTEAPP_BROWSER_ICE_TRANSPORT_POLICY \
  --env EASYNET_REMOTEAPP_BROWSER_ALLOWED_OUTBOUND_ICE_CANDIDATE_TYPES \
  --env EASYNET_REMOTEAPP_BROWSER_ALLOWED_INBOUND_ICE_CANDIDATE_TYPES \
  "$BROWSER_IMAGE" \
  bash -lc 'chrome_path="${EASYNET_REMOTEAPP_BROWSER_CHROME_PATH:-$(find /ms-playwright -type f -path "*/chrome-linux/chrome" | head -n 1)}"; test -n "$chrome_path"; EASYNET_REMOTEAPP_BROWSER_CHROME_PATH="$chrome_path" node scripts/remoteapp-browser-lifecycle.mjs' \
  >"$browser_dir/runner.stdout.txt" 2>"$browser_dir/runner.stderr.txt" &
BROWSER_DOCKER_PID=$!
browser_deadline_epoch=$(( $(date +%s) + BROWSER_RUN_DEADLINE_SECONDS ))
browser_timed_out=0
while kill -0 "$BROWSER_DOCKER_PID" >/dev/null 2>&1; do
  if (( $(date +%s) >= browser_deadline_epoch )); then
    browser_timed_out=1
    kill "$BROWSER_DOCKER_PID" >/dev/null 2>&1 || true
    bounded_stop_container "$BROWSER_CONTAINER"
    break
  fi
  sleep 1
done
if [[ "$browser_timed_out" -eq 1 ]]; then
  wait "$BROWSER_DOCKER_PID" >/dev/null 2>&1 || true
  BROWSER_DOCKER_PID=""
  write_status failed "Browser container exceeded the ${BROWSER_RUN_DEADLINE_SECONDS}s proof deadline"
  exit 1
fi
if ! wait "$BROWSER_DOCKER_PID"; then
  BROWSER_DOCKER_PID=""
  write_status failed "Browser lifecycle runner exited before producing a valid STUN proof"
  exit 1
fi
BROWSER_DOCKER_PID=""

python3 "$PROJECTOR" \
  --browser-evidence "$browser_dir/evidence.json" \
  --route-kind stun_srflx \
  --constraints-applied-at-ms "$constraints_applied_at_ms" \
  --binding-log "$STUN_EVENT_LOG" \
  --output "$OUT_DIR/evidence.json"

"$VERIFIER" --run --required-routes stun_srflx \
  --evidence-json "$OUT_DIR/evidence.json" --out-dir "$OUT_DIR/verified" >/dev/null
cp "$OUT_DIR/verified/report.json" "$OUT_DIR/report.json"
echo "[host-remoteapp-stun-srflx-e2e] PASS: $OUT_DIR/report.json"
