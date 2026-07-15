from easynet_sdk import (
    AddressingClient,
    AxonAddressingTransport,
    DaemonControl,
    DaemonMode,
    RuntimeHandle,
    RuntimeHostRole,
    RuntimeLifecycle,
    RuntimeLifecycleState,
    RuntimeAbilityClient,
    RuntimeAdminClient,
    RuntimeAdminAbilityClient,
    RuntimeCallContext,
    RuntimeClient,
    RuntimeDeviceRevokeRequest,
    RuntimeSessionListRequest,
    SDKError,
    StartConfig,
)
from easynet_sdk.health import HealthClient


class MemoryDaemonTransport:
    def __init__(self) -> None:
        self.status_json = (
            b'{"handle_id":"daemon-1","state":"Running","mode":"hub",'
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

    def invoke(self, draft_json: bytes) -> bytes:
        import json

        self.seen = json.loads(draft_json)
        return json.dumps(
            {
                "ok": True,
                "tuple": self.seen,
                "invocation_id": "inv-admin-test",
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_json": self.output_json,
                "elapsed_ms": 1,
                "error": None,
            }
        ).encode()


def test_runtime_admin_readiness_composes_lifecycle_and_health() -> None:
    admin = RuntimeAdminClient(
        RuntimeLifecycle(MemoryDaemonTransport()),
        HealthClient(MemoryHealthTransport()),
    )

    handle = admin.start(StartConfig(mode=RuntimeHostRole.HUB))
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


def test_runtime_lifecycle_names_keep_daemon_alias_compatibility() -> None:
    assert DaemonControl is RuntimeLifecycle
    assert DaemonMode is RuntimeHostRole


def test_runtime_admin_ability_client_lists_sessions() -> None:
    client, transport = _ability_client()
    transport.output_json = {
        "state": "ok",
        "sessions": [
            {
                "kind": "terminal",
                "session_id": "session-a",
                "device_ura": "easynet:///r/example/device/laptop",
                "hub_ura": "easynet:///r/example/hub",
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
        "easynet:///r/example/ability/hub.session.list@1.0.0"
    )
    assert transport.seen["args"]["include_terminated"] is False
    assert transport.seen["metadata"]["sdk_profile"] == "runtime_admin"
    assert transport.seen["metadata"]["system_ability"] == "session.list"


def test_runtime_admin_ability_client_revokes_device() -> None:
    client, transport = _ability_client()
    transport.output_json = {"ack": False, "runtime_not_ready": True}

    result = client.revoke_device(
        RuntimeDeviceRevokeRequest(
            call=_call(),
            device_ura="easynet:///r/example/device/laptop",
            reason="owner_removed_device",
        )
    )

    assert result.ack is False
    assert result.runtime_not_ready is True
    assert transport.seen["descriptor_ref"] == (
        "easynet:///r/example/ability/hub.federation.revoke@1.0.0"
    )
    assert transport.seen["args"] == {
        "agent_ura": "easynet:///r/example/device/laptop",
        "reason": "owner_removed_device",
    }


def test_runtime_admin_ability_client_rejects_missing_revoke_ack() -> None:
    client, transport = _ability_client()
    transport.output_json = {"runtime_not_ready": False}

    try:
        client.revoke_device(
            RuntimeDeviceRevokeRequest(
                call=_call(),
                device_ura="easynet:///r/example/device/laptop",
                reason="owner_removed_device",
            )
        )
    except SDKError as exc:
        assert "ack must be a boolean" in exc.message
    else:
        raise AssertionError("revoke_device fabricated success without ack")


def test_runtime_admin_ability_client_rejects_malformed_revoke_flags() -> None:
    client, transport = _ability_client()
    transport.output_json = {"ack": True, "runtime_not_ready": "false"}

    try:
        client.revoke_device(
            RuntimeDeviceRevokeRequest(
                call=_call(),
                device_ura="easynet:///r/example/device/laptop",
                reason="owner_removed_device",
            )
        )
    except SDKError as exc:
        assert "runtime_not_ready must be a boolean" in exc.message
    else:
        raise AssertionError("revoke_device accepted malformed readiness flag")


def test_runtime_admin_ability_client_rejects_invalid_revoke_before_invoke() -> None:
    client, transport = _ability_client()

    try:
        client.revoke_device(
            RuntimeDeviceRevokeRequest(
                call=_call(),
                device_ura="easynet:///r/example/device/laptop",
                reason="",
            )
        )
    except SDKError as exc:
        assert "device_ura and reason are required" in exc.message
    else:
        raise AssertionError("revoke_device accepted missing reason")
    assert transport.seen == {}


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
        callee_ura="easynet:///r/example/hub",
        subject_ura="easynet:///r/example/resource/device.laptop/invoke/backend.admin",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        metadata={"request_id": "admin-test"},
    )
