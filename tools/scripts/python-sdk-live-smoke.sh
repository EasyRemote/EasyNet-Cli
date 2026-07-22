#!/usr/bin/env bash
# python-sdk-live-smoke.sh — live daemon smoke through the Python SDK facade
# =========================================================================
#
# Builds generic C ABI v6 and the complete daemon process set, starts a hermetic daemon through
# `easynet_sdk.CABIRuntimeLifecycleTransport`, then exercises Runtime Core unary, stream,
# stream, prepare/sign/submit, and typed terminal failure paths through the Python SDK
# object model.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

if [[ "${1:-}" == "--self-test" ]]; then
  bash -n "$0"
  grep -q "generic C ABI v6" "$0"
  grep -q "typed terminal failure decoded" "$0"
  grep -q "RuntimeEventClient read live daemon handle events" "$0"
  grep -q "select_python_bin" "$0"
  grep -q "using Python interpreter" "$0"
  grep -q "install uv/python3" "$0"
  grep -q "EXPECTED_ABI_VERSION = 6" "$REPO_ROOT/sdk/python/easynet_sdk/_cabi.py"
  grep -q "def open_cabi_runtime_lifecycle_transport" "$REPO_ROOT/sdk/python/easynet_sdk/_cabi.py"
  grep -q "class RuntimeClient" "$REPO_ROOT/sdk/python/easynet_sdk/runtime.py"
  echo "python-sdk-live-smoke self-test ok"
  exit 0
fi

select_python_bin() {
  if [[ -n "${PYTHON_BIN:-}" ]]; then
    [[ -x "$PYTHON_BIN" ]] || {
      echo "[python-sdk-live-smoke] PYTHON_BIN is not executable: $PYTHON_BIN" >&2
      exit 2
    }
    return
  fi

  local sdk_venv_python="$REPO_ROOT/sdk/python/.venv/bin/python"
  if [[ -x "$sdk_venv_python" ]]; then
    PYTHON_BIN="$sdk_venv_python"
    return
  fi

  if command -v uv >/dev/null 2>&1; then
    (cd "$REPO_ROOT/sdk/python" && uv sync --quiet)
    if [[ -x "$sdk_venv_python" ]]; then
      PYTHON_BIN="$sdk_venv_python"
      return
    fi
    echo "[python-sdk-live-smoke] uv sync did not create an executable interpreter at $sdk_venv_python" >&2
    exit 2
  fi

  if command -v python3 >/dev/null 2>&1; then
    PYTHON_BIN="$(command -v python3)"
    return
  fi

  echo "[python-sdk-live-smoke] no Python interpreter found; set PYTHON_BIN or install uv/python3" >&2
  exit 2
}

select_python_bin
echo "[python-sdk-live-smoke] using Python interpreter: $PYTHON_BIN"
DAEMON_BIN="$REPO_ROOT/target/debug/easynet-daemon"

case "$(uname -s)" in
  Darwin) LIB_EXT="dylib" ;;
  Linux) LIB_EXT="so" ;;
  *)
    echo "[python-sdk-live-smoke] unsupported OS: $(uname -s)" >&2
    exit 2
    ;;
esac

LIB_PATH="$REPO_ROOT/target/debug/libeasynet_cli.${LIB_EXT}"

echo "[python-sdk-live-smoke] rebuilding libeasynet_cli + daemon process set..."
"$REPO_ROOT/tools/scripts/build-daemon-process-set.sh" --lib

SMOKE_HOME="$(mktemp -d "/tmp/easynet-python-sdk-smoke.XXXXXX")"
cleanup() {
  status=$?
  if [[ "$status" -ne 0 ]]; then
    echo "[python-sdk-live-smoke] FAIL: dumping hermetic daemon log from $SMOKE_HOME" >&2
    if [[ -f "$SMOKE_HOME/.easynet/python-sdk-smoke-daemon.log" ]]; then
      tail -n 160 "$SMOKE_HOME/.easynet/python-sdk-smoke-daemon.log" >&2 || true
    else
      find "$SMOKE_HOME" -maxdepth 3 -type f -print >&2 || true
    fi
  fi
  rm -rf "$SMOKE_HOME"
}
trap cleanup EXIT

echo "[python-sdk-live-smoke] starting daemon through Python SDK facade..."
PYTHONPATH="$REPO_ROOT/sdk/python:$REPO_ROOT/../EasyNet-Axon/sdk/python" \
LIB_PATH="$LIB_PATH" \
DAEMON_BIN="$DAEMON_BIN" \
SMOKE_HOME="$SMOKE_HOME" \
"$PYTHON_BIN" - <<'PY'
import base64
import json
import os
import time
from pathlib import Path

from easynet_sdk import (
    HealthClient,
    InvocationSignature,
    RuntimeEventClient,
    RuntimeEventReadRequest,
    RuntimeEventStreamState,
    RuntimeHandleEventProvider,
    RuntimeLifecycle,
)
from easynet_sdk._cabi import CABIRuntimeLifecycleTransport, CLILibrary
from easynet_sdk.errors import SDKError
from easynet_sdk.providers.easynet.lifecycle import DaemonMode, StartConfig


def wait_until(label, predicate, timeout_s=8.0):
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        if predicate():
            return
        time.sleep(0.02)
    raise AssertionError(f"timed out waiting for {label}")


def write_hermetic_identity(smoke_home):
    state_dir = Path(smoke_home) / ".easynet"
    state_dir.mkdir(parents=True, exist_ok=True)
    realm = "cli"
    device_id = "local"
    device_ura = f"easynet:///r/{realm}/device/{device_id}"
    invocation_socket = "~/.easynet/custom-invocation.sock"

    (state_dir / "credentials.json").write_text(
        json.dumps(
            {
                "node_id": device_id,
                "credential_token": "python-sdk-smoke-token",
                "hub_endpoint": "https://127.0.0.1:50443",
                "realm": realm,
                "username": "python-sdk-smoke-user",
                "user_id": "python-sdk-smoke-user-id",
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    (state_dir / "daemon-config.toml").write_text(
        f'''[daemon]
mode = "device"
realm = "{realm}"
hub_endpoint = "https://127.0.0.1:50443"
uds_path = "{invocation_socket}"
''',
        encoding="utf-8",
    )
    fake_public_key_b64 = base64.b64encode(bytes([1]) * 32).decode("ascii")
    trust_path = state_dir / "realm-trust.toml"
    trust_path.write_text(
        f'''[[trusted_agent]]
agent_ura = "{device_ura}"
public_key_b64 = "{fake_public_key_b64}"
role = "device"
added_at_unix_ms = 0
''',
        encoding="utf-8",
    )
    return realm, device_id, device_ura, str(trust_path)


def nonce(start):
    return base64.b64encode(bytes(range(start, start + 16))).decode("ascii")


def draft(runtime, device_ura, ability, args, nonce_start, call_mode="rpc"):
    descriptor_ref = runtime.resolve_descriptor_ref(
        callee_ura=device_ura,
        ability=ability,
        call_mode=call_mode,
        caller_ura=device_ura,
        subject_ura=device_ura,
    )
    return (
        runtime.new_invocation()
        .with_caller_ura(device_ura)
        .with_callee_ura(device_ura)
        .with_descriptor_ref(descriptor_ref)
        .with_subject_ura(device_ura)
        .with_nonce_base64(nonce(nonce_start))
        .with_causal_context({"form": "none"})
        .with_content_type("application/json")
        .with_json_args(args)
        .build()
    )


smoke_home = os.environ["SMOKE_HOME"]
realm, device_id, device_ura, trust_path = write_hermetic_identity(smoke_home)
os.environ["HOME"] = smoke_home
os.environ["EASYNET_REALM_TRUST_PATH"] = trust_path
os.environ["EASYNET_PAGES_PORT"] = str(19000 + (os.getpid() % 1000))

transport = CABIRuntimeLifecycleTransport(CLILibrary.load(os.environ["LIB_PATH"]))
control = RuntimeLifecycle(transport)
handle = None
runtime = None
try:
    handle = control.start(
        StartConfig(
            mode=DaemonMode.DEVICE,
            realm=realm,
            device_id=device_id,
            daemon_bin=os.environ["DAEMON_BIN"],
            log_path=str(Path(smoke_home) / ".easynet" / "python-sdk-smoke-daemon.log"),
            env={
                "HOME": smoke_home,
                "EASYNET_REALM_TRUST_PATH": trust_path,
                "EASYNET_PAGES_PORT": os.environ["EASYNET_PAGES_PORT"],
            },
        )
    )
    status = handle.status()
    assert status.endpoints.invocation_endpoint, status
    assert handle.invocation_endpoint(), status
    runtime = handle.open_runtime()

    health = HealthClient(runtime._transport).runtime_health()
    assert health.ready(), health

    unary = runtime.invoke(
        draft(runtime, device_ura, "observe.health", {"smoke": "python-sdk"}, 1)
    )
    assert unary.ok is True, unary
    assert unary.terminal_state == "Completed", unary
    assert unary.output_json["status"] == "healthy", unary.output_json
    assert unary.output_json["echo"]["smoke"] == "python-sdk", unary.output_json
    print("[python-sdk-live-smoke] unary RuntimeClient.invoke OK")

    try:
        runtime.resolve_descriptor_ref(
            callee_ura=device_ura,
            ability="sdk.live_smoke_missing",
            call_mode="rpc",
            caller_ura=device_ura,
            subject_ura=device_ura,
        )
        raise AssertionError("unknown ability unexpectedly resolved descriptor_ref")
    except SDKError as exc:
        assert exc.code == "DESCRIPTOR_NOT_FOUND", exc
    print("[python-sdk-live-smoke] missing ability descriptor resolve fails closed")

    prepared_failure, _ = runtime.prepare(
        draft(
            runtime,
            device_ura,
            "observe.health",
            {"smoke": "python-sdk-terminal-failure"},
            65,
        )
    )
    signed_failure = prepared_failure.sign_with_caller_signature(
        InvocationSignature(
            algorithm="ed25519",
            signature_base64="c2lnbmF0dXJl",
            key_id_hint="python-sdk-live-smoke-invalid-signature",
        )
    )
    failure_handle = runtime.submit_signed(signed_failure)
    terminal_failure = runtime.await_result(failure_handle)
    assert terminal_failure.ok is False, terminal_failure
    assert terminal_failure.terminal_state == "Failed", terminal_failure
    assert terminal_failure.error is not None, terminal_failure
    assert terminal_failure.error.code, terminal_failure.error
    assert terminal_failure.error.stage, terminal_failure.error
    assert terminal_failure.error.message, terminal_failure.error
    print(
        "[python-sdk-live-smoke] typed terminal failure decoded: "
        f"code={terminal_failure.error.code} stage={terminal_failure.error.stage}"
    )

    event_client = RuntimeEventClient(RuntimeHandleEventProvider(runtime))
    event_page = event_client.read(
        RuntimeEventReadRequest(handle=failure_handle, limit=8)
    )
    assert event_page.terminal is True, event_page
    assert event_page.state is RuntimeEventStreamState.TERMINAL, event_page
    assert len(event_page.events) > 0, event_page
    last_event = event_page.events[-1]
    assert last_event.terminal is True, last_event
    assert last_event.state == "Failed", last_event
    assert event_page.cursor.sequence == last_event.sequence, event_page
    print("[python-sdk-live-smoke] RuntimeEventClient read live daemon handle events")

    stream = runtime.invoke_stream(
        draft(
            runtime,
            device_ura,
            "session.attach",
            {"session_id": "python-sdk-live-smoke-no-such-session"},
            33,
            call_mode="stream",
        )
    )
    stream_event = stream.next(timeout=5.0)
    assert stream_event.kind == "terminal", stream_event
    assert stream_event.terminal is True, stream_event
    assert stream_event.terminal_receipt is not None, stream_event
    stream.close()
    print("[python-sdk-live-smoke] StreamHandle received receipt-backed daemon terminal frame")

finally:
    if runtime is not None:
        runtime.close()
    if handle is not None:
        handle.stop()
    transport.close()

print("[python-sdk-live-smoke] PASS")
PY
