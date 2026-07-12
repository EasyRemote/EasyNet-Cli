#!/usr/bin/env bash
# backend-live-principal-e2e.sh — Backend account flow against a live daemon

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
BACKEND_ROOT="${EASYNET_BACKEND_ROOT:-$REPO_ROOT/../EasyNet/backend}"
DAEMON_BIN="$REPO_ROOT/target/debug/easynet-daemon"

case "$(uname -s)" in
  Darwin) LIB_EXT="dylib" ;;
  Linux) LIB_EXT="so" ;;
  *)
    echo "[backend-live-principal-e2e] unsupported OS: $(uname -s)" >&2
    exit 2
    ;;
esac

LIB_PATH="$REPO_ROOT/target/debug/libeasynet_cli.${LIB_EXT}"

if [[ "${1:-}" == "--self-test" ]]; then
  bash -n "$0"
  test -f "$BACKEND_ROOT/internal/logic/user/register_user_signing_key_live_daemon_test.go"
  grep -q "backend_live_daemon" "$BACKEND_ROOT/internal/logic/user/register_user_signing_key_live_daemon_test.go"
  grep -q "TestRegisterUserSigningKey_BackendAccountFlowUsesLiveDaemonPrincipalLifecycle" "$BACKEND_ROOT/internal/logic/user/register_user_signing_key_live_daemon_test.go"
  grep -q "OpenCABIDaemonTransport" "$BACKEND_ROOT/internal/logic/user/register_user_signing_key_live_daemon_test.go"
  grep -q "principalprofile.NewClient" "$BACKEND_ROOT/internal/logic/user/register_user_signing_key_live_daemon_test.go"
  echo "backend-live-principal-e2e self-test ok"
  exit 0
fi

if [[ ! -f "$BACKEND_ROOT/go.mod" ]]; then
  echo "[backend-live-principal-e2e] backend go.mod not found at $BACKEND_ROOT" >&2
  exit 2
fi

echo "[backend-live-principal-e2e] rebuilding libeasynet_cli + easynet-daemon..."
(cd "$REPO_ROOT" && cargo build --lib --bin easynet-daemon)

SMOKE_HOME="$(mktemp -d "/tmp/easynet-backend-live-principal.XXXXXX")"
cleanup() {
  status=$?
  if [[ "$status" -ne 0 ]]; then
    echo "[backend-live-principal-e2e] FAIL: dumping hermetic daemon log from $SMOKE_HOME" >&2
    if [[ -f "$SMOKE_HOME/.easynet/backend-live-daemon.log" ]]; then
      tail -n 180 "$SMOKE_HOME/.easynet/backend-live-daemon.log" >&2 || true
    else
      find "$SMOKE_HOME" -maxdepth 3 -type f -print >&2 || true
    fi
  fi
  rm -rf "$SMOKE_HOME"
}
trap cleanup EXIT

echo "[backend-live-principal-e2e] running Backend live daemon PrincipalLifecycle E2E..."
(
  cd "$BACKEND_ROOT"
  CGO_ENABLED=1 \
  EASYNET_BACKEND_LIVE_DAEMON_LIB="$LIB_PATH" \
  EASYNET_BACKEND_LIVE_DAEMON_BIN="$DAEMON_BIN" \
  EASYNET_BACKEND_LIVE_DAEMON_HOME="$SMOKE_HOME" \
  go test -tags "easynet_cabi backend_live_daemon" ./internal/logic/user -run '^TestRegisterUserSigningKey_BackendAccountFlowUsesLiveDaemonPrincipalLifecycle$' -count=1 -v
)

echo "[backend-live-principal-e2e] PASS"
