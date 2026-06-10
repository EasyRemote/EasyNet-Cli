#!/usr/bin/env bash
# daemon-ability-smoke.sh — local daemon Axon ability smoke test
# ==============================================================
#
# Boots `easynet-daemon`, waits for the daemon-hosted Axon
# Invocation socket, and invokes a canonical observe.health
# Ability URA through the CLI's ability surface. This deliberately
# avoids control.sock and does not construct legacy control-plane frames.

set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
DAEMON_BIN="$REPO_ROOT/target/debug/easynet-daemon"
CLI_BIN="$REPO_ROOT/target/debug/easynet"
DAEMON_SOCK="${EASYNET_DAEMON_GRPC_UDS:-$HOME/.easynet/daemon.sock}"
KEEP_DAEMON=0

for arg in "$@"; do
  case "$arg" in
    --keep-daemon) KEEP_DAEMON=1 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

if [ ! -x "$DAEMON_BIN" ] || [ ! -x "$CLI_BIN" ]; then
  echo "[smoke] building easynet + easynet-daemon (debug, product defaults)..."
  (cd "$REPO_ROOT" && cargo build --bin easynet --bin easynet-daemon)
fi

pkill -f "$DAEMON_BIN" 2>/dev/null || true
rm -f "$DAEMON_SOCK"

echo "[smoke] starting daemon..."
"$DAEMON_BIN" &
DAEMON_PID=$!
trap '[ "$KEEP_DAEMON" -eq 0 ] && kill "$DAEMON_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 30); do
  [ -S "$DAEMON_SOCK" ] && break
  sleep 0.2
done
if [ ! -S "$DAEMON_SOCK" ]; then
  echo "[smoke] FAIL: socket did not appear at $DAEMON_SOCK" >&2
  exit 1
fi

ABILITY_URA="easynet:///r/cli/ability/device.local.observe.health"
echo "[smoke] invoking $ABILITY_URA through daemon-hosted Axon..."
RESP="$("$CLI_BIN" ability invoke "$ABILITY_URA" --args '{"smoke":"ok"}' --raw)"
echo "[smoke] response: $RESP"

RESP="$RESP" python3 - <<'PY'
import json, os
r = json.loads(os.environ["RESP"])
assert r.get("echo", {}).get("smoke") == "ok", r
assert "replied_at_unix_ms" in r, r
print("[smoke] PASS")
PY
