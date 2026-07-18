#!/usr/bin/env bash
# backend-live-http-daemon-e2e.sh — browser HTTP to real daemon contract gate

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
BACKEND_ROOT="${EASYNET_BACKEND_ROOT:-$REPO_ROOT/../EasyNet/backend}"
DAEMON_BIN="$REPO_ROOT/target/debug/easynet-daemon"

case "$(uname -s)" in
  Darwin) LIB_EXT="dylib" ;;
  Linux) LIB_EXT="so" ;;
  *) echo "[backend-live-http-daemon-e2e] unsupported OS: $(uname -s)" >&2; exit 2 ;;
esac

if [[ "${1:-}" == "--self-test" ]]; then
  bash -n "$0"
  test -f "$BACKEND_ROOT/internal/handler/bridge_http_live_daemon_e2e_test.go"
  grep -q "TestBridgeHTTP_E2E_RegisteredBrowserInvokesHubAbilityThroughLiveDaemon" \
    "$BACKEND_ROOT/internal/handler/bridge_http_live_daemon_e2e_test.go"
  echo "backend-live-http-daemon-e2e self-test ok"
  exit 0
fi

LIB_PATH="$REPO_ROOT/target/debug/libeasynet_cli.${LIB_EXT}"
echo "[backend-live-http-daemon-e2e] rebuilding libeasynet_cli + daemon process set..."
"$REPO_ROOT/tools/scripts/build-daemon-process-set.sh" --lib

echo "[backend-live-http-daemon-e2e] running browser HTTP → live daemon E2E..."
(
  cd "$BACKEND_ROOT"
  CGO_ENABLED=1 \
  EASYNET_BACKEND_LIVE_DAEMON_LIB="$LIB_PATH" \
  EASYNET_BACKEND_LIVE_DAEMON_BIN="$DAEMON_BIN" \
  go test -tags "easynet_cabi backend_live_daemon" ./internal/handler \
    -run '^TestBridgeHTTP_E2E_RegisteredBrowserInvokesHubAbilityThroughLiveDaemon$' \
    -count=1 -v
)

echo "[backend-live-http-daemon-e2e] PASS"
