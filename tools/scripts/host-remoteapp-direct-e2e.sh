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

DAEMON_WAS_RUNNING=0
pgrep -f '/easynet-daemon$' >/dev/null 2>&1 && DAEMON_WAS_RUNNING=1
RESTORE_FAILED=0

cleanup() {
  local exit_code=$?
  "$RUNTIME_CLI" runtime stop >/dev/null 2>&1 || true
  if [[ "$DAEMON_WAS_RUNNING" -eq 1 ]]; then
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
"$RUNTIME_CLI" runtime stop >"$OUT_DIR/runtime-stop.stdout.txt" 2>"$OUT_DIR/runtime-stop.stderr.txt" || true
env \
  -u EASYNET_REMOTE_DESKTOP_STUN_URLS \
  -u EASYNET_REMOTE_DESKTOP_TURN_URLS \
  -u EASYNET_REMOTE_DESKTOP_TURN_USERNAME \
  -u EASYNET_REMOTE_DESKTOP_TURN_CREDENTIAL \
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
  EASYNET_REMOTEAPP_BROWSER_ICE_TRANSPORT_POLICY=all \
  node "$BROWSER_RUNNER"
) >"$browser_dir/runner.stdout.txt" 2>"$browser_dir/runner.stderr.txt"

python3 "$PROJECTOR" \
  --browser-evidence "$browser_dir/evidence.json" \
  --route-kind direct \
  --constraints-applied-at-ms "$constraints_applied_at_ms" \
  --output "$OUT_DIR/evidence.json"

"$VERIFIER" --run --required-routes direct \
  --evidence-json "$OUT_DIR/evidence.json" --out-dir "$OUT_DIR/verified" >/dev/null
cp "$OUT_DIR/verified/report.json" "$OUT_DIR/report.json"
echo "[host-remoteapp-direct-e2e] PASS: $OUT_DIR/report.json"
