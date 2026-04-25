#!/usr/bin/env bash
# ffi-smoke.sh — load libeasynet_cli via ctypes, exercise the C ABI
# ==================================================================
#
# Boots `easynet-daemon`, then loads `libeasynet_cli` via Python
# ctypes, calls the exported C ABI symbols (init / ability_invoke /
# shutdown), and asserts the daemon's v1 skeleton response surfaces
# with the right error code (`ERR_ABILITY_FAILED = 9`) and the
# documented message via `easynet_last_error()`.
#
# Why a separate smoke from `control-smoke.sh`
# --------------------------------------------
# `control-smoke.sh` exercises the wire (raw UDS frames). This
# script exercises the cdylib (C ABI). They overlap on the daemon
# response but cover different failure modes:
#
#   - control-smoke catches "wire/codec/server" regressions.
#   - ffi-smoke catches "C ABI shape / cbindgen / handle registry /
#     last-error TLS" regressions.
#
# Both are fast (under a second) and run in CI as a pair.
#
# Usage:
#   scripts/ffi-smoke.sh

set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
DAEMON_BIN="$REPO_ROOT/target/debug/easynet-daemon"

# Library extension differs by OS; pick the right one. Linux has
# .so, macOS .dylib, Windows .dll (the latter is not supported by
# this script — Windows uses a separate PowerShell smoke).
case "$(uname -s)" in
  Darwin) LIB_EXT="dylib" ;;
  Linux)  LIB_EXT="so"    ;;
  *)
    echo "[ffi-smoke] unsupported OS for ctypes smoke: $(uname -s)" >&2
    exit 2
    ;;
esac
LIB_PATH="$REPO_ROOT/target/debug/libeasynet_cli.${LIB_EXT}"

if [ ! -f "$LIB_PATH" ]; then
  echo "[ffi-smoke] building libeasynet_cli (debug)..."
  (cd "$REPO_ROOT" && cargo build --lib)
fi
if [ ! -x "$DAEMON_BIN" ]; then
  echo "[ffi-smoke] building easynet-daemon (debug)..."
  (cd "$REPO_ROOT" && cargo build --bin easynet-daemon)
fi

# Same daemon-cleanup dance as control-smoke.sh: kill any prior
# instance and remove the stale socket file before booting.
pkill -f "$DAEMON_BIN" 2>/dev/null || true
rm -f "$HOME/.easynet/control.sock"

echo "[ffi-smoke] starting daemon..."
"$DAEMON_BIN" &
DAEMON_PID=$!
trap 'kill "$DAEMON_PID" 2>/dev/null || true' EXIT

# Wait up to 2 s for the socket to appear.
for _ in $(seq 1 20); do
  [ -S "$HOME/.easynet/control.sock" ] && break
  sleep 0.1
done
if [ ! -S "$HOME/.easynet/control.sock" ]; then
  echo "[ffi-smoke] FAIL: socket did not appear" >&2
  exit 1
fi

echo "[ffi-smoke] loading libeasynet_cli via ctypes and exercising C ABI..."

LIB_PATH="$LIB_PATH" python3 - <<'PY'
import ctypes, ctypes.util, os, sys

lib_path = os.environ["LIB_PATH"]
lib = ctypes.CDLL(lib_path)

# Signatures match include/easynet_cli.h — abbreviated here because
# Python ctypes is not generated from cbindgen.
lib.easynet_abi_version.restype = ctypes.c_uint32
lib.easynet_abi_version.argtypes = []

lib.easynet_init.restype = ctypes.c_int32
lib.easynet_init.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_uint64)]

lib.easynet_shutdown.restype = ctypes.c_int32
lib.easynet_shutdown.argtypes = [ctypes.c_uint64]

lib.easynet_ability_invoke.restype = ctypes.c_int32
lib.easynet_ability_invoke.argtypes = [
    ctypes.c_uint64,
    ctypes.c_char_p,
    ctypes.c_char_p,
    ctypes.POINTER(ctypes.c_char_p),
]

lib.easynet_last_error.restype = ctypes.c_char_p
lib.easynet_last_error.argtypes = []

lib.easynet_string_free.restype = None
lib.easynet_string_free.argtypes = [ctypes.c_char_p]

# 1. ABI version sanity.
ver = lib.easynet_abi_version()
assert ver == 1, f"unexpected ABI version: {ver}"
print(f"[ffi-smoke] ABI version: {ver}")

# 2. easynet_init with default control.json (NULL path).
handle = ctypes.c_uint64(0)
rc = lib.easynet_init(None, ctypes.byref(handle))
assert rc == 0, (
    f"easynet_init returned {rc}; "
    f"last_error={lib.easynet_last_error()}"
)
assert handle.value != 0, "init returned OK but handle is 0"
print(f"[ffi-smoke] init OK; handle={handle.value}")

# 3. easynet_ability_invoke with system.ping.
# PR-INVOCATION-EXEC-UNITY wired the proxy through the real
# dispatcher, so we now expect EASYNET_OK (0) and a non-NULL
# result string. The result JSON shape is owned by the ping
# handler; we do not pin the value bytes here, only the ABI-
# level invariants (rc=0, out_result populated, freeable).
out_ptr = ctypes.c_char_p()
rc = lib.easynet_ability_invoke(
    handle, b"system.ping", b"{}", ctypes.byref(out_ptr)
)
assert rc == 0, (
    f"expected EASYNET_OK (0), got {rc}; "
    f"last_error={lib.easynet_last_error()}"
)
assert out_ptr.value, "out_result must be a non-NULL CString on the OK path"
result_json = out_ptr.value.decode("utf-8")
print(f"[ffi-smoke] invoke OK; result={result_json!r}")
# The ABI requires the caller to free the heap-allocated CString
# via easynet_string_free; otherwise valgrind / asan would catch
# a per-call leak in production.
lib.easynet_string_free(out_ptr)

# 4. easynet_shutdown — handle was registered, so this returns OK.
rc = lib.easynet_shutdown(handle.value)
assert rc == 0, f"easynet_shutdown returned {rc}"
print(f"[ffi-smoke] shutdown OK")

# 5. Idempotency: a second shutdown must report ERR_INVALID_HANDLE (4).
rc = lib.easynet_shutdown(handle.value)
assert rc == 4, f"second shutdown should be ERR_INVALID_HANDLE (4), got {rc}"
print(f"[ffi-smoke] double-shutdown returns ERR_INVALID_HANDLE as expected")

print("[ffi-smoke] PASS")
PY
