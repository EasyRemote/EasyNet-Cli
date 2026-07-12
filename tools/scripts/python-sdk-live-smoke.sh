#!/usr/bin/env bash
# python-sdk-live-smoke.sh — live daemon smoke through the Python SDK facade
# =========================================================================
#
# Builds generic C ABI v5 `libeasynet_cli` and `easynet-daemon`, starts a hermetic daemon through
# `easynet_sdk.CABIDaemonTransport`, then exercises Runtime Core unary, stream,
# stream, prepare/sign/submit, and typed terminal failure paths through the Python SDK
# object model.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

if [[ "${1:-}" == "--self-test" ]]; then
  bash -n "$0"
  grep -q "generic C ABI v5" "$0"
  grep -q "typed terminal failure decoded" "$0"
  grep -q "RuntimeEventClient read live daemon handle events" "$0"
  grep -q "EXPECTED_ABI_VERSION = 5" "$REPO_ROOT/sdk/python/easynet_sdk/_cabi.py"
  grep -q "class DaemonControl" "$REPO_ROOT/sdk/python/easynet_sdk/daemon.py"
  grep -q "class RuntimeClient" "$REPO_ROOT/sdk/python/easynet_sdk/runtime.py"
  echo "python-sdk-live-smoke self-test ok"
  exit 0
fi

if [[ -z "${PYTHON_BIN:-}" ]]; then
  command -v uv >/dev/null 2>&1 || {
    echo "[python-sdk-live-smoke] uv is required to materialize the SDK test environment" >&2
    exit 2
  }
  (cd "$REPO_ROOT/sdk/python" && uv sync --quiet)
  PYTHON_BIN="$REPO_ROOT/sdk/python/.venv/bin/python"
fi
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

echo "[python-sdk-live-smoke] rebuilding libeasynet_cli + easynet-daemon..."
(cd "$REPO_ROOT" && cargo build --lib --bin easynet-daemon)

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
    DaemonControl,
    DaemonMode,
    HealthClient,
    InvocationSignature,
    RuntimeEventClient,
    RuntimeEventReadRequest,
    RuntimeEventStreamState,
    RuntimeHandleEventProvider,
    StartConfig,
)
from easynet_sdk._cabi import CABIDaemonTransport, CLILibrary


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


def draft(runtime, device_ura, ability, args, nonce_start):
    realm, device_id = device_ura.removeprefix("easynet:///r/").split("/device/", 1)
    descriptor_ref = (
        f"easynet:///r/{realm}/ability/device.{device_id}.{ability}@1.0.0"
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

transport = CABIDaemonTransport(CLILibrary.load(os.environ["LIB_PATH"]))
control = DaemonControl(transport)
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

    prepared_failure, _ = runtime.prepare(
        draft(
            runtime,
            device_ura,
            "sdk.live_smoke_missing",
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

    browser = runtime.invoke(
        draft(
            runtime,
            device_ura,
            "browser.open_session",
            {"url": "https://example.com"},
            17,
        )
    )
    assert browser.ok is True, browser
    session_ura = browser.output_json["session_ura"]
    stream = runtime.invoke_stream(
        draft(
            runtime,
            device_ura,
            "browser.capture_viewport",
            {"session_ura": session_ura},
            33,
        )
    )
    stream_event = stream.next(timeout=5.0)
    assert stream_event.kind == "chunk", stream_event
    assert stream_event.payload_json["is_placeholder"] is True, stream_event.payload_json
    stream.cancel("python-sdk-live-smoke")
    print("[python-sdk-live-smoke] StreamHandle received daemon frame")

finally:
    if runtime is not None:
        runtime.close()
    if handle is not None:
        handle.stop()
    transport.close()

print("[python-sdk-live-smoke] PASS")
PY
