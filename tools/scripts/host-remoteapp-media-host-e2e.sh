#!/usr/bin/env bash
# Real Linux/X11 process proof for RemoteApp media-host active sessions.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_NAME="real_x11_window_and_application_sessions_emit_recoverable_h264"

if [[ "${1:-}" == "--self-test" ]]; then
  bash -n "$0"
  grep -Fq 'EASYNET_REMOTEAPP_SENTINEL_FIXTURE' "$0"
  grep -Fq 'easynet-remoteapp-media-host' "$0"
  grep -Fq 'real_x11_window_and_application_sessions_emit_recoverable_h264' "$0"
  echo 'host-remoteapp-media-host-e2e self-test ok'
  exit 0
fi

[[ "$(uname -s)" == Linux ]] || {
  echo 'host-remoteapp-media-host-e2e requires Linux/X11' >&2
  exit 1
}
for executable in cargo python3 Xvfb openbox xdpyinfo; do
  command -v "$executable" >/dev/null || {
    echo "host-remoteapp-media-host-e2e missing executable: $executable" >&2
    exit 1
  }
done
python3 -c 'import tkinter' >/dev/null 2>&1 || {
  echo 'host-remoteapp-media-host-e2e requires Python tkinter bindings' >&2
  exit 1
}

RUN_ROOT="$(mktemp -d)"
DISPLAY_NUMBER="${EASYNET_REMOTEAPP_MEDIA_HOST_E2E_DISPLAY:-}"
XVFB_PID=''
OPENBOX_PID=''
cleanup() {
  if [[ -n "$OPENBOX_PID" ]]; then kill "$OPENBOX_PID" >/dev/null 2>&1 || true; fi
  if [[ -n "$XVFB_PID" ]]; then kill "$XVFB_PID" >/dev/null 2>&1 || true; fi
  find "$RUN_ROOT" -depth -delete 2>/dev/null || true
}
trap cleanup EXIT

if [[ -n "$DISPLAY_NUMBER" ]]; then
  Xvfb "$DISPLAY_NUMBER" -screen 0 1280x800x24 -ac +extension RANDR \
    >"$RUN_ROOT/xvfb.log" 2>&1 &
else
  Xvfb -displayfd 3 -screen 0 1280x800x24 -ac +extension RANDR \
    3>"$RUN_ROOT/display-number" >"$RUN_ROOT/xvfb.log" 2>&1 &
fi
XVFB_PID="$!"
if [[ -z "$DISPLAY_NUMBER" ]]; then
  for _ in $(seq 1 100); do
    if [[ -s "$RUN_ROOT/display-number" ]]; then break; fi
    if ! kill -0 "$XVFB_PID" >/dev/null 2>&1; then
      echo "Xvfb exited before allocating a display; see $RUN_ROOT/xvfb.log" >&2
      exit 1
    fi
    sleep 0.05
  done
  [[ -s "$RUN_ROOT/display-number" ]] || {
    echo 'Xvfb did not allocate a display number' >&2
    exit 1
  }
  IFS= read -r DISPLAY_NUMBER <"$RUN_ROOT/display-number"
  DISPLAY_NUMBER=":$DISPLAY_NUMBER"
fi
for _ in $(seq 1 100); do
  if DISPLAY="$DISPLAY_NUMBER" xdpyinfo >/dev/null 2>&1; then break; fi
  sleep 0.05
done
DISPLAY="$DISPLAY_NUMBER" xdpyinfo >/dev/null
DISPLAY="$DISPLAY_NUMBER" openbox >"$RUN_ROOT/openbox.log" 2>&1 &
OPENBOX_PID="$!"

DISPLAY="$DISPLAY_NUMBER" \
EASYNET_REMOTEAPP_SENTINEL_FIXTURE="$ROOT/tools/fixtures/remoteapp-linux-x11-sentinel.py" \
cargo test --offline -p easynet-remoteapp-media-host \
  --test linux_x11_process "$TEST_NAME" -- --ignored --exact --nocapture

echo 'host-remoteapp-media-host-e2e: ok'
