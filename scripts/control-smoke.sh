#!/usr/bin/env bash
# control-smoke.sh — local IPC plane smoke test
# ==============================================
#
# Boots `easynet-daemon`, dials the UDS at ~/.easynet/control.sock,
# sends one length-prefixed `Invoke` frame, and asserts the daemon
# returns a structured response. The v1 skeleton response is an
# `Error` envelope with code `ability_failed`; once
# PR-INVOCATION-EXEC-UNITY lands real dispatch this will switch to a
# successful `Result` envelope and the assertion below moves with it.
#
# Why a shell + python harness instead of `socat`?
# ------------------------------------------------
# `socat` cannot prefix a 4-byte LE length header without a wrapper
# anyway, so reaching for python3 is no extra dependency on macOS /
# Linux dev boxes (both ship it). The script is debugging
# infrastructure — `cargo test --lib services::control` already
# exercises the same path through the in-process server harness.
#
# Usage:
#   scripts/control-smoke.sh [--keep-daemon]
#
#   --keep-daemon   leave the daemon running after the test (default
#                   is to SIGTERM it on exit so the next invocation
#                   has a clean ~/.easynet/control.sock).

set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
DAEMON_BIN="$REPO_ROOT/target/debug/easynet-daemon"
KEEP_DAEMON=0

for arg in "$@"; do
  case "$arg" in
    --keep-daemon) KEEP_DAEMON=1 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

if [ ! -x "$DAEMON_BIN" ]; then
  echo "[smoke] building easynet-daemon (debug)..."
  (cd "$REPO_ROOT" && cargo build --bin easynet-daemon)
fi

# Kill any stale daemon from a previous run; the UDS file would
# otherwise stick around (bind_at clears it but a live process would
# block bind).
pkill -f "$DAEMON_BIN" 2>/dev/null || true
rm -f "$HOME/.easynet/control.sock"

echo "[smoke] starting daemon..."
"$DAEMON_BIN" &
DAEMON_PID=$!
trap '[ "$KEEP_DAEMON" -eq 0 ] && kill "$DAEMON_PID" 2>/dev/null || true' EXIT

# Wait up to 2 s for the socket to appear.
for _ in $(seq 1 20); do
  [ -S "$HOME/.easynet/control.sock" ] && break
  sleep 0.1
done
if [ ! -S "$HOME/.easynet/control.sock" ]; then
  echo "[smoke] FAIL: socket did not appear at ~/.easynet/control.sock" >&2
  exit 1
fi

echo "[smoke] dialing socket and sending one Invoke frame..."
RESP="$(
python3 - <<'PY'
import socket, struct, json, os, sys
sock_path = os.path.expanduser("~/.easynet/control.sock")
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sock_path)
req = {"type": "invoke", "request_id": "smoke-1", "ability": "system.ping", "args": {}}
payload = json.dumps(req).encode()
s.sendall(struct.pack("<I", len(payload)) + payload)
raw_len = s.recv(4)
(resp_len,) = struct.unpack("<I", raw_len)
buf = b""
while len(buf) < resp_len:
    chunk = s.recv(resp_len - len(buf))
    if not chunk: break
    buf += chunk
sys.stdout.write(buf.decode())
PY
)"

echo "[smoke] response: $RESP"

# PR-INVOCATION-EXEC-UNITY post-condition: system.ping dispatches
# through the real registry, so we expect a `result` envelope with
# `request_id` round-tripped. The exact `value` shape is owned by
# the ping handler — do not pin the value bytes here, only the
# wire-level invariants that Client bindings depend on.
echo "$RESP" | python3 -c '
import json, sys
r = json.loads(sys.stdin.read())
assert r.get("type") == "result", f"expected type=result, got {r}"
assert r.get("request_id") == "smoke-1", f"request_id mismatch: {r}"
assert "value" in r, f"missing value field: {r}"
print("[smoke] PASS")
'
