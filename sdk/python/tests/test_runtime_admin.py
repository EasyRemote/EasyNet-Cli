import hashlib
from pathlib import Path

from easynet_sdk import (
    AddressingClient,
    AxonAddressingTransport,
    ErrorCode,
    RuntimeHandle,
    RuntimeLifecycle,
    RuntimeLifecycleState,
    RuntimeAbilityClient,
    RuntimeAdminClient,
    RuntimeAdminAbilityClient,
    RuntimeCallContext,
    RuntimeClient,
    RuntimeSessionListRequest,
    RuntimeStatus,
    SDKError,
)
from easynet_sdk.providers.runtime.lifecycle import (
    RuntimeHostMode,
    RuntimeHostStartConfig,
)
from easynet_sdk._runtime_admin_routes import (
    _PROFILE as _RUNTIME_ADMIN_PROFILE,
    _RUNTIME_ADMIN_ROUTE_MANIFEST_SHA256,
    _RUNTIME_ADMIN_SESSION_LIST_ABILITY,
)
from easynet_sdk.health import HealthClient
from test_runtime import canonical_runtime_receipt_pair


def test_runtime_admin_routes_are_generated_from_manifest() -> None:
    manifest = (
        Path(__file__).resolve().parents[2].parent
        / "provider_routes"
        / "easynet-runtime-admin-routes.v1.json"
    )
    digest = hashlib.sha256(manifest.read_bytes()).hexdigest()
    assert _RUNTIME_ADMIN_ROUTE_MANIFEST_SHA256 == digest


def test_runtime_status_rejects_retired_product_mode() -> None:
    try:
        RuntimeStatus.from_json(
            b'{"handle_id":"daemon-1","state":"Running","mode":"hub",'
            b'"endpoints":{"invocation_endpoint":"unix:///tmp/daemon.sock"}}'
        )
    except SDKError as exc:
        assert exc.code == ErrorCode.INVALID_ARGUMENT
        assert "invalid runtime host mode" in exc.message
    else:
        raise AssertionError("RuntimeStatus accepted retired product mode")


class MemoryDaemonTransport:
    def __init__(self) -> None:
        self.status_json = (
            b'{"handle_id":"daemon-1","state":"Running","mode":"authority",'
            b'"endpoints":{"control_endpoint":"unix:///tmp/control.sock",'
            b'"invocation_endpoint":"unix:///tmp/daemon.sock"},'
            b'"diagnostics":["status-ok"]}'
        )

    def discover(self, options_json: bytes) -> bytes:
        return (
            b'{"control_endpoint":"unix:///tmp/control.sock",'
            b'"invocation_endpoint":"unix:///tmp/daemon.sock"}'
        )

    def start(self, config_json: bytes) -> bytes:
        return self.status_json

    def attach(self, options_json: bytes) -> bytes:
        return self.status_json

    def status(self, handle_id: str) -> bytes:
        return self.status_json

    def invocation_endpoint(self, handle_id: str) -> str:
        return "unix:///tmp/daemon.sock"

    def open_runtime(self, handle_id: str, options_json: bytes):
        from test_runtime import MemoryRuntimeTransport

        return MemoryRuntimeTransport(), b'{"ready":true}'

    def stop(self, handle_id: str, options_json: bytes) -> bytes:
        return b'{"handle_id":"daemon-1","state":"Stopped","diagnostics":[]}'

    def detach(self, handle_id: str) -> None:
        return None


class MemoryHealthTransport:
    def runtime_health(self) -> bytes:
        return (
            b'{"api_ready":true,"invocation_ready":true,"directory_ready":true,'
            b'"trust_ready":true,"runtime_ready":true,'
            b'"diagnostics":["health-ok"]}'
        )

    def runtime_diagnostics(self) -> bytes:
        return (
            b'{"profile":"health","kind":"diagnostics_report","state":"Running",'
            b'"ready":true,"version":"0.91.30","abi_version":5,'
            b'"control_endpoint":"unix:///tmp/control.sock",'
            b'"invocation_endpoint":"unix:///tmp/daemon.sock",'
            b'"checks":[{"name":"runtime","ready":true,"message":null}],'
            b'"diagnostics":[]}'
        )


class RuntimeAdminTransportFake:
    def __init__(self) -> None:
        self.seen: dict[str, object] = {}
        self.output_json: dict[str, object] = {"state": "ok"}

    def resolve_descriptor_ref(self, request_json: bytes) -> bytes:
        import json

        request = json.loads(request_json)
        return json.dumps(
            {
                "descriptor_ref": (
                    f"easynet:///r/example/ability/authority.{request['ability']}@1.0.0"
                )
            }
        ).encode()

    def invoke(self, draft_json: bytes) -> bytes:
        import json

        self.seen = json.loads(draft_json)
        admission, terminal = canonical_runtime_receipt_pair("inv-admin-test")
        return json.dumps(
            {
                "ok": True,
                "tuple": self.seen,
                "invocation_id": "inv-admin-test",
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_json": self.output_json,
                "elapsed_ms": 1,
                "admission_receipt": admission,
                "terminal_receipt": terminal,
                "error": None,
            }
        ).encode()


def test_runtime_admin_readiness_composes_lifecycle_and_health() -> None:
    admin = RuntimeAdminClient(
        RuntimeLifecycle(MemoryDaemonTransport()),
        HealthClient(MemoryHealthTransport()),
    )

    handle = admin.start(RuntimeHostStartConfig(mode=RuntimeHostMode.AUTHORITY))
    readiness = admin.readiness(handle)

    assert isinstance(handle, RuntimeHandle)
    assert readiness.ready is True
    assert readiness.lifecycle_state == RuntimeLifecycleState.RUNNING
    assert readiness.messages == ("status-ok", "health-ok")
    assert readiness.diagnostics is not None


def test_runtime_admin_rejects_missing_handle_and_control() -> None:
    admin = RuntimeAdminClient(RuntimeLifecycle(MemoryDaemonTransport()))

    try:
        admin.status(None)  # type: ignore[arg-type]
    except SDKError as exc:
        assert "runtime handle is required" in exc.message
    else:
        raise AssertionError("status accepted missing handle")

    try:
        RuntimeAdminClient(None)  # type: ignore[arg-type]
    except SDKError as exc:
        assert "runtime lifecycle is required" in exc.message
    else:
        raise AssertionError("constructor accepted missing control")


def test_runtime_admin_ability_client_lists_sessions() -> None:
    client, transport = _ability_client()
    transport.output_json = {
        "state": "ok",
        "sessions": [
            {
                "kind": "terminal",
                "session_id": "session-a",
                "device_ura": "easynet:///r/example/device/laptop",
                "hub_ura": "easynet:///r/example/authority",
                "state": "active",
                "session_kind": "pty",
                "created_unix_ms": 1714492800000,
                "expires_unix_ms": 1714496400000,
                "metadata": {"source": "daemon"},
            }
        ],
    }

    page = client.list_sessions(
        RuntimeSessionListRequest(call=_call(), include_terminated=False)
    )

    assert len(page.sessions) == 1
    assert page.sessions[0].session_id == "session-a"
    assert transport.seen["descriptor_ref"] == (
        "easynet:///r/example/ability/authority.session.list@1.0.0"
    )
    assert transport.seen["args"]["include_terminated"] is False
    assert transport.seen["metadata"]["sdk_profile"] == _RUNTIME_ADMIN_PROFILE
    assert (
        transport.seen["metadata"]["system_ability"]
        == _RUNTIME_ADMIN_SESSION_LIST_ABILITY
    )


def test_runtime_admin_ability_client_accepts_empty_sessions() -> None:
    client, transport = _ability_client()
    transport.output_json = {"state": "ok", "sessions": []}

    page = client.list_sessions(RuntimeSessionListRequest(call=_call()))

    assert page.sessions == ()


def test_runtime_admin_ability_client_rejects_legacy_session_items() -> None:
    client, transport = _ability_client()
    transport.output_json = {"state": "ok", "items": []}

    try:
        client.list_sessions(RuntimeSessionListRequest(call=_call()))
    except SDKError as exc:
        assert "sessions must be an array" in exc.message
    else:
        raise AssertionError("list_sessions accepted legacy items fallback")


def test_runtime_admin_ability_client_rejects_malformed_session_rows() -> None:
    client, transport = _ability_client()
    transport.output_json = {"state": "ok", "sessions": ["bad-row"]}

    try:
        client.list_sessions(RuntimeSessionListRequest(call=_call()))
    except SDKError as exc:
        assert "sessions entries must be objects" in exc.message
    else:
        raise AssertionError("list_sessions ignored malformed session row")


def test_runtime_admin_ability_client_does_not_expose_device_revoke_surface() -> None:
    import easynet_sdk

    client, _transport = _ability_client()
    assert not hasattr(client, "revoke_device")
    for retired in ("RuntimeDeviceRevokeRequest", "RuntimeDeviceRevokeResult"):
        assert not hasattr(easynet_sdk, retired), retired


def _ability_client() -> tuple[RuntimeAdminAbilityClient, RuntimeAdminTransportFake]:
    transport = RuntimeAdminTransportFake()
    ability = RuntimeAbilityClient(
        RuntimeClient(transport),  # type: ignore[arg-type]
        AddressingClient(AxonAddressingTransport()),
    )
    return RuntimeAdminAbilityClient(ability), transport


def _call() -> RuntimeCallContext:
    return RuntimeCallContext(
        caller_ura="easynet:///r/example/agent/backend",
        callee_ura="easynet:///r/example/authority",
        subject_ura="easynet:///r/example/resource/device.laptop/invoke/backend.admin",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        metadata={"request_id": "admin-test"},
    )
