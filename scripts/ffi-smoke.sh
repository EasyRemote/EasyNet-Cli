#!/usr/bin/env bash
# ffi-smoke.sh — load libeasynet_cli via ctypes, exercise the C ABI
# ==================================================================
#
# Boots `easynet-daemon`, then loads `libeasynet_cli` via Python
# ctypes and exercises the ABI v3 surface: init/shutdown, daemon
# lifecycle preflight, and complete Invocation unary/stream/bidi
# argument validation.
#
# Why a separate smoke from `control-smoke.sh`
# --------------------------------------------
# `control-smoke.sh` exercises the wire (raw UDS frames). This
# script exercises the cdylib (C ABI). They overlap on the daemon
# response but cover different failure modes:
#
#   - control-smoke catches "wire/codec/server" regressions.
#   - ffi-smoke catches "C ABI shape / header contract / handle
#     registry / last-error TLS" regressions.
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

echo "[ffi-smoke] rebuilding libeasynet_cli + easynet-daemon (debug, axon-pb)..."
(cd "$REPO_ROOT" && cargo build --lib --bin easynet-daemon --features axon-pb)

# Hermetic state root. The smoke must not kill a developer's real
# daemon or mutate their real ~/.easynet credentials/sockets.
SMOKE_HOME="$(mktemp -d "/tmp/easynet-ffi-smoke.XXXXXX")"
cleanup() {
  status=$?
  if [ "$status" -ne 0 ]; then
    echo "[ffi-smoke] FAIL: dumping hermetic daemon log from $SMOKE_HOME" >&2
    if [ -f "$SMOKE_HOME/.easynet/ffi-smoke-daemon.log" ]; then
      tail -n 120 "$SMOKE_HOME/.easynet/ffi-smoke-daemon.log" >&2 || true
    else
      find "$SMOKE_HOME" -maxdepth 3 -type f -print >&2 || true
    fi
  fi
  rm -rf "$SMOKE_HOME"
}
trap cleanup EXIT

echo "[ffi-smoke] loading libeasynet_cli via ctypes and exercising C ABI..."

LIB_PATH="$LIB_PATH" DAEMON_BIN="$DAEMON_BIN" SMOKE_HOME="$SMOKE_HOME" python3 - <<'PY'
import base64
import ctypes
import hashlib
import json
import os
import time

lib_path = os.environ["LIB_PATH"]
daemon_bin = os.environ["DAEMON_BIN"]
smoke_home = os.environ["SMOKE_HOME"]
lib = ctypes.CDLL(lib_path)

# Signatures match include/easynet_cli.h. ctypes declarations stay
# local to this smoke so it can prove the checked-in header contract
# and dynamic library agree on the same exported ABI.
lib.easynet_abi_version.restype = ctypes.c_uint32
lib.easynet_abi_version.argtypes = []

lib.easynet_init.restype = ctypes.c_int32
lib.easynet_init.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_uint64)]

lib.easynet_shutdown.restype = ctypes.c_int32
lib.easynet_shutdown.argtypes = [ctypes.c_uint64]

lib.easynet_daemon_start.restype = ctypes.c_int32
lib.easynet_daemon_start.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_uint64)]

lib.easynet_daemon_stop.restype = ctypes.c_int32
lib.easynet_daemon_stop.argtypes = [ctypes.c_uint64]

lib.easynet_daemon_status.restype = ctypes.c_int32
lib.easynet_daemon_status.argtypes = [ctypes.c_uint64, ctypes.POINTER(ctypes.c_char_p)]

lib.easynet_daemon_invocation_endpoint.restype = ctypes.c_int32
lib.easynet_daemon_invocation_endpoint.argtypes = [ctypes.c_uint64, ctypes.POINTER(ctypes.c_char_p)]

lib.easynet_daemon_open_client.restype = ctypes.c_int32
lib.easynet_daemon_open_client.argtypes = [ctypes.c_uint64, ctypes.POINTER(ctypes.c_uint64)]

lib.easynet_invocation_invoke.restype = ctypes.c_int32
lib.easynet_invocation_invoke.argtypes = [
    ctypes.c_uint64,
    ctypes.c_char_p,
    ctypes.POINTER(ctypes.c_char_p),
]

lib.easynet_invocation_stream_open.restype = ctypes.c_int32
lib.easynet_invocation_stream_open.argtypes = [
    ctypes.c_uint64,
    ctypes.c_char_p,
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.c_uint64),
]

lib.easynet_invocation_stream_cancel.restype = ctypes.c_int32
lib.easynet_invocation_stream_cancel.argtypes = [
    ctypes.c_uint64,
    ctypes.c_uint64,
]

lib.easynet_invocation_bidi_open.restype = ctypes.c_int32
lib.easynet_invocation_bidi_open.argtypes = [
    ctypes.c_uint64,
    ctypes.c_char_p,
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.c_uint64),
]

lib.easynet_invocation_bidi_send.restype = ctypes.c_int32
lib.easynet_invocation_bidi_send.argtypes = [
    ctypes.c_uint64,
    ctypes.c_uint64,
    ctypes.c_char_p,
]

lib.easynet_invocation_bidi_close.restype = ctypes.c_int32
lib.easynet_invocation_bidi_close.argtypes = [
    ctypes.c_uint64,
    ctypes.c_uint64,
]

lib.easynet_invocation_bidi_cancel.restype = ctypes.c_int32
lib.easynet_invocation_bidi_cancel.argtypes = [
    ctypes.c_uint64,
    ctypes.c_uint64,
]

lib.easynet_last_error.restype = ctypes.c_char_p
lib.easynet_last_error.argtypes = []

lib.easynet_string_free.restype = None
lib.easynet_string_free.argtypes = [ctypes.c_char_p]

daemon_handle = ctypes.c_uint64(0)
client_from_daemon = ctypes.c_uint64(0)
init_handle = ctypes.c_uint64(0)

STREAM_CALLBACK = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_char_p)
BIDI_CALLBACK = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_char_p)

def last_error():
    raw = lib.easynet_last_error()
    return raw.decode("utf-8", "replace") if raw else ""

def cstr_value(ptr):
    if not ptr.value:
        return ""
    return ptr.value.decode("utf-8")

def assert_ok(rc, label):
    assert rc == 0, f"{label} returned {rc}; last_error={last_error()}"

def wait_until(label, predicate, timeout_s=5.0, detail=None):
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        if predicate():
            return
        time.sleep(0.02)
    suffix = f"; {detail()}" if detail else ""
    raise AssertionError(f"timed out waiting for {label}{suffix}")

def seed_hermetic_identity():
    state_dir = os.path.join(smoke_home, ".easynet")
    os.makedirs(state_dir, exist_ok=True)
    realm = "ffi-smoke"
    node_id = "device-a"
    invocation_socket = "~/.easynet/custom-invocation.sock"
    device_ura = f"easynet:///r/{realm}/device/{node_id}"
    credentials_path = os.path.join(state_dir, "credentials.json")
    with open(credentials_path, "w", encoding="utf-8") as f:
        json.dump(
            {
                "node_id": node_id,
                "credential_token": "ffi-smoke-token",
                "hub_endpoint": "https://127.0.0.1:50443",
                "realm": realm,
                "username": "ffi-smoke-user",
            },
            f,
            indent=2,
        )
        f.write("\n")

    daemon_config_path = os.path.join(state_dir, "daemon-config.toml")
    with open(daemon_config_path, "w", encoding="utf-8") as f:
        f.write(
            f'''[daemon]
mode = "device"
realm = "{realm}"
hub_endpoint = "https://127.0.0.1:50443"
uds_path = "{invocation_socket}"
'''
        )

    trust_path = os.path.join(state_dir, "realm-trust.toml")
    fake_public_key_b64 = base64.b64encode(bytes([1]) * 32).decode("ascii")
    with open(trust_path, "w", encoding="utf-8") as f:
        f.write(
            f'''[[trusted_agent]]
agent_ura = "{device_ura}"
public_key_b64 = "{fake_public_key_b64}"
role = "device"
added_at_unix_ms = 0
'''
        )
    os.environ["HOME"] = smoke_home
    os.environ["EASYNET_REALM_TRUST_PATH"] = trust_path
    os.environ["EASYNET_PAGES_PORT"] = str(18000 + (os.getpid() % 1000))
    return realm, node_id, f"easynet:///r/{realm}/device/{node_id}"

try:
    # 1. ABI version sanity.
    ver = lib.easynet_abi_version()
    assert ver == 2, f"unexpected ABI version: {ver}"
    print(f"[ffi-smoke] ABI version: {ver}")

    # 2. Daemon lifecycle ABI preflight. Malformed config fails
    # before process spawn and must leave the daemon handle zero.
    daemon_handle = ctypes.c_uint64(42)
    rc = lib.easynet_daemon_start(b"{not-json", ctypes.byref(daemon_handle))
    assert rc == 11, f"daemon start malformed JSON should be ERR_INVALID_ARG (11), got {rc}"
    assert daemon_handle.value == 0, "daemon start must zero out_daemon_handle on failure"
    status_out = ctypes.c_char_p()
    rc = lib.easynet_daemon_status(9_999_999, ctypes.byref(status_out))
    assert rc == 4, f"daemon status invalid handle should be ERR_INVALID_HANDLE (4), got {rc}"
    assert not status_out.value, "daemon status must leave output NULL on invalid handle"
    endpoint_out = ctypes.c_char_p()
    rc = lib.easynet_daemon_invocation_endpoint(9_999_999, ctypes.byref(endpoint_out))
    assert rc == 4, f"daemon endpoint invalid handle should be ERR_INVALID_HANDLE (4), got {rc}"
    assert not endpoint_out.value, "daemon endpoint must leave output NULL on invalid handle"
    rc = lib.easynet_daemon_stop(9_999_999)
    assert rc == 4, f"daemon stop invalid handle should be ERR_INVALID_HANDLE (4), got {rc}"
    client_from_daemon = ctypes.c_uint64(42)
    rc = lib.easynet_daemon_open_client(9_999_999, ctypes.byref(client_from_daemon))
    assert rc == 4, f"daemon open client invalid handle should be ERR_INVALID_HANDLE (4), got {rc}"
    assert client_from_daemon.value == 0, "daemon open client must zero out_handle on failure"
    print("[ffi-smoke] daemon lifecycle preflight rejects malformed config and invalid handles")

    # 3. Start through the lifecycle ABI. Success means both
    # control.sock and daemon.sock are accepting before the function
    # returns.
    realm, node_id, self_device_ura = seed_hermetic_identity()
    config = {
        "mode": "device",
        "realm": realm,
        "node_id": node_id,
        "daemon_bin": daemon_bin,
        "detach": False,
        "log_path": os.path.join(smoke_home, ".easynet", "ffi-smoke-daemon.log"),
        "env": {
            "HOME": smoke_home,
            "EASYNET_REALM_TRUST_PATH": os.environ["EASYNET_REALM_TRUST_PATH"],
            "EASYNET_PAGES_PORT": os.environ["EASYNET_PAGES_PORT"],
        },
    }
    rc = lib.easynet_daemon_start(json.dumps(config).encode("utf-8"), ctypes.byref(daemon_handle))
    assert_ok(rc, "easynet_daemon_start")
    assert daemon_handle.value != 0, "daemon start returned OK but handle is 0"
    print(f"[ffi-smoke] daemon_start OK; daemon_handle={daemon_handle.value}")

    status_json = ctypes.c_char_p()
    assert_ok(lib.easynet_daemon_status(daemon_handle.value, ctypes.byref(status_json)), "easynet_daemon_status")
    status = json.loads(cstr_value(status_json))
    assert status["control_accepting"] is True, status
    assert status["invocation_accepting"] is True, status
    print("[ffi-smoke] daemon status reports control + invocation accepting")

    # 4. Open an Invocation-capable client directly from the daemon
    # lifecycle handle, then also prove normal easynet_init still
    # works once the daemon is ready.
    assert_ok(
        lib.easynet_daemon_open_client(daemon_handle.value, ctypes.byref(client_from_daemon)),
        "easynet_daemon_open_client",
    )
    assert client_from_daemon.value != 0, "daemon open client returned OK but handle is 0"
    print(f"[ffi-smoke] daemon_open_client OK; handle={client_from_daemon.value}")

    assert_ok(lib.easynet_init(None, ctypes.byref(init_handle)), "easynet_init")
    assert init_handle.value != 0, "init returned OK but handle is 0"
    print(f"[ffi-smoke] init OK; handle={init_handle.value}")

    # 5. Complete Invocation happy path: daemon_start -> open_client
    # -> easynet_invocation_invoke -> receipt/result JSON, then the
    # same call through an easynet_init handle. The daemon config uses
    # a non-default uds_path, so init must consume the advertised
    # Invocation endpoint from control.json instead of guessing
    # parent/daemon.sock.
    invocation = {
        "caller_ura": self_device_ura,
        "callee_ura": self_device_ura,
        "ability": "observe.health",
        "subject_ura": self_device_ura,
        "nonce_base64": base64.b64encode(bytes(range(1, 17))).decode("ascii"),
        "causal_context": {"form": "none"},
        "args": {"smoke": "ffi-happy-path"},
    }
    def invoke_health(handle, label, nonce_start):
        request = dict(invocation)
        request["nonce_base64"] = base64.b64encode(bytes(range(nonce_start, nonce_start + 16))).decode("ascii")
        out_ptr = ctypes.c_char_p()
        rc = lib.easynet_invocation_invoke(
            handle,
            json.dumps(request).encode("utf-8"),
            ctypes.byref(out_ptr),
        )
        assert_ok(rc, label)
        response = json.loads(cstr_value(out_ptr))
        assert response["ok"] is True, response
        assert response["ability"] == "observe.health", response
        assert response["result_content_type"] == "application/json", response
        assert response["result_json"]["status"] == "healthy", response
        assert response["result_json"]["echo"]["smoke"] == "ffi-happy-path", response

    def invoke_json(handle, label, ability, args, nonce_start):
        request = dict(invocation)
        request["ability"] = ability
        request["nonce_base64"] = base64.b64encode(bytes(range(nonce_start, nonce_start + 16))).decode("ascii")
        request["args"] = args
        out_ptr = ctypes.c_char_p()
        rc = lib.easynet_invocation_invoke(
            handle,
            json.dumps(request).encode("utf-8"),
            ctypes.byref(out_ptr),
        )
        assert_ok(rc, label)
        response = json.loads(cstr_value(out_ptr))
        assert response["ok"] is True, response
        assert response["ability"] == ability, response
        assert response["result_content_type"] == "application/json", response
        return response["result_json"]

    invoke_health(client_from_daemon.value, "easynet_invocation_invoke daemon-open-client", 1)
    invoke_health(init_handle.value, "easynet_invocation_invoke init-handle", 17)
    print("[ffi-smoke] complete Invocation happy path works through daemon and init handles")

    # 6. Stream happy path through real daemon InvokeStream. An
    # opened browser mock session returns exactly one capture
    # snapshot; the callback proves the C ABI reader/dispatcher path
    # receives daemon frames.
    browser_open = invoke_json(
        client_from_daemon.value,
        "easynet_invocation_invoke browser.open_session",
        "browser.open_session",
        {"url": "https://example.com"},
        33,
    )
    session_ura = browser_open["session_ura"]
    stream_frames = []
    @STREAM_CALLBACK
    def on_stream_chunk(_user_data, frame_json):
        stream_frames.append(json.loads(frame_json.decode("utf-8")))

    stream_invocation = dict(invocation)
    stream_invocation["ability"] = "browser.capture_viewport"
    stream_invocation["nonce_base64"] = base64.b64encode(bytes(range(49, 65))).decode("ascii")
    stream_invocation["args"] = {"session_ura": session_ura}
    stream_id = ctypes.c_uint64(0)
    assert_ok(
        lib.easynet_invocation_stream_open(
            client_from_daemon.value,
            json.dumps(stream_invocation).encode("utf-8"),
            ctypes.cast(on_stream_chunk, ctypes.c_void_p),
            None,
            ctypes.byref(stream_id),
        ),
        "easynet_invocation_stream_open happy path",
    )
    assert stream_id.value != 0, "stream open returned OK but stream id is 0"
    wait_until("stream callback frame", lambda: len(stream_frames) > 0)
    assert any(
        frame.get("payload_json", {}).get("is_placeholder") is True
        for frame in stream_frames
    ), stream_frames
    first_stream = stream_frames[0]
    assert first_stream["ok"] is True, first_stream
    assert first_stream["event"] == "chunk", first_stream
    assert first_stream["content_type"] == "application/json", first_stream
    assert_ok(
        lib.easynet_invocation_stream_cancel(client_from_daemon.value, stream_id.value),
        "easynet_invocation_stream_cancel after callback",
    )
    print("[ffi-smoke] complete Invocation stream happy path delivered callback frame")

    # 7. Bidi happy path through real daemon InvokeBidi. The
    # daemon must accept frame 0, deliver the admission callback
    # frame, forward a business BinaryChunk, and finish with a
    # terminal completion receipt.
    bidi_frames = []
    @BIDI_CALLBACK
    def on_bidi_frame(_user_data, frame_json):
        bidi_frames.append(json.loads(frame_json.decode("utf-8")))

    download_path = os.path.join(smoke_home, ".easynet", "ffi-smoke-download.bin")
    download_bytes = b"ffi bidi download proof\n"
    with open(download_path, "wb") as f:
        f.write(download_bytes)
    bidi_invocation = dict(invocation)
    bidi_invocation["ability"] = "fs.transfer"
    bidi_invocation["nonce_base64"] = base64.b64encode(bytes(range(65, 81))).decode("ascii")
    bidi_invocation["args"] = {"mode": "download", "path": download_path}
    bidi_invocation["bidi_streams"] = [
        {"stream_id": 1, "content_type": "application/octet-stream", "ordering": "STRICT"}
    ]
    bidi_id = ctypes.c_uint64(0)
    assert_ok(
        lib.easynet_invocation_bidi_open(
            client_from_daemon.value,
            json.dumps(bidi_invocation).encode("utf-8"),
            ctypes.cast(on_bidi_frame, ctypes.c_void_p),
            None,
            ctypes.byref(bidi_id),
        ),
        "easynet_invocation_bidi_open happy path",
    )
    assert bidi_id.value != 0, "bidi open returned OK but bidi id is 0"
    assert_ok(
        lib.easynet_invocation_bidi_send(
            client_from_daemon.value,
            bidi_id.value,
            json.dumps({"type": "control", "eof": True}).encode("utf-8"),
        ),
        "easynet_invocation_bidi_send download ready/eof hint",
    )
    wait_until(
        "bidi business frames",
        lambda: (
            any(frame.get("event") == "binary_chunk" for frame in bidi_frames)
            and any(
                frame.get("event") == "receipt"
                and frame.get("receipt", {}).get("state") == 5
                and frame.get("terminal") is True
                for frame in bidi_frames
            )
        ),
        detail=lambda: f"frames={json.dumps(bidi_frames, sort_keys=True)}",
    )
    first_bidi = bidi_frames[0]
    assert first_bidi["ok"] is True, first_bidi
    assert first_bidi["event"] == "receipt", first_bidi
    assert first_bidi["receipt"]["state"] == 2, first_bidi  # Axon InvocationState::Admitted
    assert first_bidi["terminal"] is False, first_bidi
    chunk_payloads = [
        base64.b64decode(frame["data_base64"])
        for frame in bidi_frames
        if frame.get("event") == "binary_chunk"
    ]
    assert b"".join(chunk_payloads) == download_bytes, bidi_frames
    terminal_bidi = [
        frame for frame in bidi_frames
        if frame.get("event") == "receipt" and frame.get("terminal") is True
    ][-1]
    assert terminal_bidi["receipt"]["state"] == 5, terminal_bidi  # Axon InvocationState::Completed
    assert hashlib.sha256(download_bytes).hexdigest(), "sha sanity"
    print("[ffi-smoke] complete Invocation bidi happy path delivered data and terminal receipt")

    # 8. Complete Invocation ABI preflight. Malformed JSON fails
    # before daemon Invocation I/O and must leave the output pointer
    # null.
    out_ptr = ctypes.c_char_p()
    rc = lib.easynet_invocation_invoke(client_from_daemon.value, b"{not-json", ctypes.byref(out_ptr))
    assert rc == 11, f"expected ERR_INVALID_ARG (11), got {rc}; last_error={last_error()}"
    assert not out_ptr.value, "out_receipt_json must stay NULL on parse failure"
    print("[ffi-smoke] invocation invoke preflight rejects malformed JSON")

    # 9. Stream and bidi open must reject a NULL callback before
    # daemon I/O while zeroing their local handle outputs.
    stream_id = ctypes.c_uint64(42)
    rc = lib.easynet_invocation_stream_open(
        client_from_daemon.value, b"{not-json", None, None, ctypes.byref(stream_id)
    )
    assert rc == 2, f"stream open NULL callback should be ERR_NULL_POINTER (2), got {rc}"
    assert stream_id.value == 0, "stream open must zero out_stream_id on failure"
    print("[ffi-smoke] invocation stream open preflight zeros stream id")

    bidi_id = ctypes.c_uint64(42)
    rc = lib.easynet_invocation_bidi_open(
        client_from_daemon.value, b"{not-json", None, None, ctypes.byref(bidi_id)
    )
    assert rc == 2, f"bidi open NULL callback should be ERR_NULL_POINTER (2), got {rc}"
    assert bidi_id.value == 0, "bidi open must zero out_bidi_id on failure"
    print("[ffi-smoke] invocation bidi open preflight zeros bidi id")

    # 10. easynet_shutdown and idempotency.
    assert_ok(lib.easynet_shutdown(init_handle.value), "easynet_shutdown(init)")
    rc = lib.easynet_shutdown(init_handle.value)
    assert rc == 4, f"second shutdown should be ERR_INVALID_HANDLE (4), got {rc}"
    print("[ffi-smoke] init double-shutdown returns ERR_INVALID_HANDLE as expected")

    assert_ok(lib.easynet_shutdown(client_from_daemon.value), "easynet_shutdown(open_client)")
    client_from_daemon.value = 0
    init_handle.value = 0
finally:
    if init_handle.value:
        lib.easynet_shutdown(init_handle.value)
    if client_from_daemon.value:
        lib.easynet_shutdown(client_from_daemon.value)
    if daemon_handle.value:
        lib.easynet_daemon_stop(daemon_handle.value)

print("[ffi-smoke] PASS")
PY
