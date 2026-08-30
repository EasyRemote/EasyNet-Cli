#!/usr/bin/env bash
# go-sdk-live-smoke.sh — live daemon smoke through the Go SDK facade
# ==================================================================
#
# Builds `libeasynet_cli` and the complete daemon process set, then runs the tagged Go SDK
# live smoke against a hermetic daemon. The test exercises daemon lifecycle,
# base C ABI v7 Runtime Core health, unary, stream, and typed terminal failure,
# plus a real ABI v9 leased payload through public Go SDK objects.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
DAEMON_BIN="$REPO_ROOT/target/debug/easynet-daemon"

case "$(uname -s)" in
  Darwin) LIB_EXT="dylib" ;;
  Linux) LIB_EXT="so" ;;
  *)
    echo "[go-sdk-live-smoke] unsupported OS: $(uname -s)" >&2
    exit 2
    ;;
esac

LIB_PATH="$REPO_ROOT/target/debug/libeasynet_cli.${LIB_EXT}"

if [[ "${1:-}" == "--self-test" ]]; then
  bash -n "$0"
  grep -q "TestGoSDKLiveDaemonSmoke" "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  grep -q "runtime_live_smoke" "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  grep -q "base C ABI v7 boundary" "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  grep -q "ABI v9 LeasedStreamHandle received raw daemon payload" "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  grep -q '"resource.watch_remote_targets"' "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  grep -q "ActiveSignerForSubject" "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  grep -q "abilityClient.OpenLeasedStream" "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  grep -q "typed terminal failure decoded" "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  grep -q "RuntimeEventClient read live daemon handle events" "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  grep -q "goLiveSmokeSystemAgentCallee" "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  grep -q "WithCallerURA(userURA)" "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  grep -q "WithCalleeURA(calleeURA)" "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  awk 'BEGIN { needle = "WithCalleeURA(deviceURA)" } index($0, needle) { found = 1 } END { exit found ? 1 : 0 }' "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  awk 'BEGIN { needle = "CalleeURA:  deviceURA" } index($0, needle) { found = 1 } END { exit found ? 1 : 0 }' "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  echo "go-sdk-live-smoke self-test ok"
  exit 0
fi

echo "[go-sdk-live-smoke] rebuilding libeasynet_cli + daemon process set..."
"$REPO_ROOT/tools/scripts/build-daemon-process-set.sh" --lib

SMOKE_HOME="$(mktemp -d "/tmp/easynet-go-sdk-smoke.XXXXXX")"
cleanup() {
  status=$?
  if [[ "$status" -ne 0 ]]; then
    echo "[go-sdk-live-smoke] FAIL: dumping hermetic daemon log from $SMOKE_HOME" >&2
    if [[ -f "$SMOKE_HOME/.easynet/go-sdk-smoke-daemon.log" ]]; then
      tail -n 160 "$SMOKE_HOME/.easynet/go-sdk-smoke-daemon.log" >&2 || true
    else
      find "$SMOKE_HOME" -maxdepth 3 -type f -print >&2 || true
    fi
  fi
  rm -rf "$SMOKE_HOME"
}
trap cleanup EXIT

echo "[go-sdk-live-smoke] running Go SDK live daemon smoke..."
(
  cd "$REPO_ROOT/sdk/go"
  CGO_ENABLED=1 \
  EASYNET_GO_LIVE_SMOKE_LIB="$LIB_PATH" \
  EASYNET_GO_LIVE_SMOKE_DAEMON="$DAEMON_BIN" \
  EASYNET_GO_LIVE_SMOKE_REPO_ROOT="$REPO_ROOT" \
  EASYNET_GO_LIVE_SMOKE_HOME="$SMOKE_HOME" \
  go test -tags "runtime_cabi runtime_live_smoke" -run '^TestGoSDKLiveDaemonSmoke$' -count=1 -v
)

echo "[go-sdk-live-smoke] PASS"
