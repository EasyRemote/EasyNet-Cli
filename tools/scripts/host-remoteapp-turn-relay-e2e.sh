#!/usr/bin/env bash
# Reproducible host Browser + daemon + coturn RemoteApp relay proof.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
FRONTEND_ROOT="${EASYNET_REMOTEAPP_TURN_E2E_FRONTEND_ROOT:-$REPO_ROOT/../EasyNet/Frontend}"
PROJECTOR="$SELF_DIR/project-remoteapp-network-scenario.py"
VERIFIER="$SELF_DIR/remoteapp-network-fallback-e2e.sh"
FIXTURE_DIR="$REPO_ROOT/tools/fixtures/remoteapp-turn-netem"
RUNTIME_CLI="${EASYNET_REMOTEAPP_TURN_E2E_RUNTIME_CLI:-$REPO_ROOT/target/debug/easynet}"
BROWSER_RUNNER="$FRONTEND_ROOT/scripts/remoteapp-browser-lifecycle.mjs"

MODE=skip
OUT_DIR="${EASYNET_REMOTEAPP_TURN_E2E_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-turn-relay/$(date -u +%Y%m%d-%H%M%S)-$$}"
FRONTEND_URL="${EASYNET_REMOTEAPP_TURN_E2E_FRONTEND_URL:-http://127.0.0.1:3000}"
DEVICE_ID="${EASYNET_REMOTEAPP_TURN_E2E_DEVICE_ID:-}"
TARGET_KIND="${EASYNET_REMOTEAPP_TURN_E2E_TARGET_KIND:-window}"
TURN_PORT="${EASYNET_REMOTEAPP_TURN_E2E_PORT:-3479}"
TURN_MIN_PORT="${EASYNET_REMOTEAPP_TURN_E2E_MIN_PORT:-49210}"
TURN_MAX_PORT="${EASYNET_REMOTEAPP_TURN_E2E_MAX_PORT:-49230}"

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-turn-relay-e2e.sh --run --device-id UUID [--out-dir DIR]
  host-remoteapp-turn-relay-e2e.sh --self-test

Required run environment:
  EASYNET_REMOTEAPP_BROWSER_EMAIL
  EASYNET_REMOTEAPP_BROWSER_PASSWORD

The runner builds a pinned coturn fixture, restarts the paired development
daemon with temporary TURN configuration, drives the real Browser RemoteApp
window flow with relay-only ICE policy, proves a server-observed allocation,
then restores the standard local daemon. It emits a focused turn_relay child
proof; it does not claim the complete four-route network matrix.
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

write_status() {
  local status="$1"
  local reason="$2"
  python3 - "$OUT_DIR/report.json" "$status" "$reason" <<'PY'
import json
import pathlib
import sys

path, status, reason = sys.argv[1:]
pathlib.Path(path).write_text(json.dumps({
    "script": "tools/scripts/host-remoteapp-turn-relay-e2e.sh",
    "status": status,
    "reason": reason,
    "coverage": {"turn_relay": status == "passed"},
    "product_complete_claim": False,
}, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

if [[ "$MODE" == "skip" ]]; then
  write_status skipped "pass --run to execute the live TURN relay scenario"
  echo "[host-remoteapp-turn-relay-e2e] skipped: $OUT_DIR/report.json"
  exit 0
fi

if [[ "$MODE" == "self-test" ]]; then
  bash -n "$0"
  python3 -m py_compile "$PROJECTOR"
  "$VERIFIER" --self-test --out-dir "$OUT_DIR/verifier-self-test" >/dev/null
  write_status passed "script syntax, projector import, and network evidence contract passed"
  echo "host-remoteapp-turn-relay-e2e self-test ok"
  exit 0
fi

for command in docker node python3 shasum openssl; do
  command -v "$command" >/dev/null 2>&1 || {
    write_status failed "required command is unavailable: $command"
    echo "required command is unavailable: $command" >&2
    exit 1
  }
done
[[ -x "$RUNTIME_CLI" ]] || { write_status failed "runtime CLI missing: $RUNTIME_CLI"; exit 1; }
[[ -f "$BROWSER_RUNNER" ]] || { write_status failed "Browser runner missing: $BROWSER_RUNNER"; exit 1; }
[[ -f "$PROJECTOR" && -x "$VERIFIER" ]] || { write_status failed "network projector/verifier missing"; exit 1; }
[[ -n "$DEVICE_ID" ]] || { write_status failed "--device-id is required"; exit 64; }
: "${EASYNET_REMOTEAPP_BROWSER_EMAIL:?EASYNET_REMOTEAPP_BROWSER_EMAIL is required}"
: "${EASYNET_REMOTEAPP_BROWSER_PASSWORD:?EASYNET_REMOTEAPP_BROWSER_PASSWORD is required}"
[[ "$TARGET_KIND" == "window" || "$TARGET_KIND" == "application" ]] || {
  write_status failed "target kind must be window or application"
  exit 64
}
for value in "$TURN_PORT" "$TURN_MIN_PORT" "$TURN_MAX_PORT"; do
  [[ "$value" =~ ^[0-9]+$ ]] && (( value > 0 && value <= 65535 )) || {
    write_status failed "TURN ports must be integers in 1..65535"
    exit 64
  }
done
(( TURN_MIN_PORT <= TURN_MAX_PORT )) || { write_status failed "TURN relay port range is invalid"; exit 64; }

TEMP_DIR="$(mktemp -d)"
CONTAINER="easynet-remoteapp-turn-$$"
DAEMON_WAS_RUNNING=0
pgrep -f '/easynet-daemon$' >/dev/null 2>&1 && DAEMON_WAS_RUNNING=1
RESTORE_FAILED=0

cleanup() {
  local exit_code=$?
  "$RUNTIME_CLI" runtime stop >/dev/null 2>&1 || true
  docker stop "$CONTAINER" >/dev/null 2>&1 || true
  if [[ "$DAEMON_WAS_RUNNING" -eq 1 ]]; then
    "$RUNTIME_CLI" runtime start >"$OUT_DIR/runtime-restore.stdout.txt" \
      2>"$OUT_DIR/runtime-restore.stderr.txt" || RESTORE_FAILED=1
  fi
  rm -rf "$TEMP_DIR"
  if [[ "$RESTORE_FAILED" -ne 0 && "$exit_code" -eq 0 ]]; then
    write_status failed "TURN proof passed but standard local daemon restore failed"
    exit 1
  fi
  exit "$exit_code"
}
trap cleanup EXIT INT TERM

fixture_hash="$(shasum -a 256 "$FIXTURE_DIR/Dockerfile" | awk '{print substr($1,1,16)}')"
fixture_image="easynet/remoteapp-turn-netem:e2e-$fixture_hash"
if ! docker image inspect "$fixture_image" >/dev/null 2>&1; then
  docker build --tag "$fixture_image" "$FIXTURE_DIR" \
    >"$OUT_DIR/fixture-build.stdout.txt" 2>"$OUT_DIR/fixture-build.stderr.txt"
fi

turn_user="easynet-e2e"
turn_credential="$(openssl rand -hex 24)"
docker run --rm -d --name "$CONTAINER" \
  -p "$TURN_PORT:$TURN_PORT/udp" -p "$TURN_PORT:$TURN_PORT/tcp" \
  -p "$TURN_MIN_PORT-$TURN_MAX_PORT:$TURN_MIN_PORT-$TURN_MAX_PORT/udp" \
  "$fixture_image" \
  --log-file=stdout --verbose --fingerprint --lt-cred-mech \
  --realm=localhost --user="$turn_user:$turn_credential" \
  --listening-port="$TURN_PORT" --min-port="$TURN_MIN_PORT" --max-port="$TURN_MAX_PORT" \
  --allow-loopback-peers '--external-ip=$(detect-external-ip)' >/dev/null

ready=0
for _ in {1..100}; do
  if docker logs "$CONTAINER" 2>&1 | grep -q 'Relay ports initialization done'; then
    ready=1
    break
  fi
  sleep 0.1
done
[[ "$ready" -eq 1 ]] || { write_status failed "coturn fixture did not become ready"; exit 1; }

constraints_applied_at_ms="$(python3 -c 'import time; print(int(time.time() * 1000))')"
"$RUNTIME_CLI" runtime stop >"$OUT_DIR/runtime-stop.stdout.txt" 2>"$OUT_DIR/runtime-stop.stderr.txt" || true
EASYNET_REMOTE_DESKTOP_TURN_URLS="turn:127.0.0.1:$TURN_PORT?transport=udp" \
EASYNET_REMOTE_DESKTOP_TURN_USERNAME="$turn_user" \
EASYNET_REMOTE_DESKTOP_TURN_CREDENTIAL="$turn_credential" \
"$RUNTIME_CLI" runtime start >"$OUT_DIR/runtime-start.stdout.txt" 2>"$OUT_DIR/runtime-start.stderr.txt"
sleep 2

browser_dir="$OUT_DIR/browser"
mkdir -p "$browser_dir"
(
  cd "$FRONTEND_ROOT"
  EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON="$browser_dir/evidence.json" \
  EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL="$FRONTEND_URL" \
  EASYNET_REMOTEAPP_BROWSER_DEVICE_ID="$DEVICE_ID" \
  EASYNET_REMOTEAPP_BROWSER_TARGET_KIND="$TARGET_KIND" \
  EASYNET_REMOTEAPP_BROWSER_ICE_TRANSPORT_POLICY=relay \
  node "$BROWSER_RUNNER"
) >"$browser_dir/runner.stdout.txt" 2>"$browser_dir/runner.stderr.txt"

docker logs "$CONTAINER" >"$TEMP_DIR/coturn.log" 2>&1
python3 "$PROJECTOR" \
  --browser-evidence "$browser_dir/evidence.json" \
  --route-kind turn_relay \
  --constraints-applied-at-ms "$constraints_applied_at_ms" \
  --allocation-log "$TEMP_DIR/coturn.log" \
  --output "$OUT_DIR/evidence.json"

"$VERIFIER" --run --required-routes turn_relay \
  --evidence-json "$OUT_DIR/evidence.json" --out-dir "$OUT_DIR/verified" >/dev/null
cp "$OUT_DIR/verified/report.json" "$OUT_DIR/report.json"
echo "[host-remoteapp-turn-relay-e2e] PASS: $OUT_DIR/report.json"
