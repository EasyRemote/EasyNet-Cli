from easynet_sdk import (
    DaemonControl,
    DaemonLifecycleState,
    DaemonMode,
    RuntimeAdminClient,
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
            b'{"api_ready":true,"daemon_ready":true,"invocation_ready":true,'
            b'"directory_ready":true,"trust_ready":true,"runtime_ready":true,'
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


def test_runtime_admin_readiness_composes_lifecycle_and_health() -> None:
    admin = RuntimeAdminClient(
        DaemonControl(MemoryDaemonTransport()),
        HealthClient(MemoryHealthTransport()),
    )

    handle = admin.start(StartConfig(mode=DaemonMode.HUB))
    readiness = admin.readiness(handle)

    assert readiness.ready is True
    assert readiness.lifecycle_state == DaemonLifecycleState.RUNNING
    assert readiness.messages == ("status-ok", "health-ok")
    assert readiness.diagnostics is not None


def test_runtime_admin_rejects_missing_handle_and_control() -> None:
    admin = RuntimeAdminClient(DaemonControl(MemoryDaemonTransport()))

    try:
        admin.status(None)  # type: ignore[arg-type]
    except SDKError as exc:
        assert "daemon handle is required" in exc.message
    else:
        raise AssertionError("status accepted missing handle")

    try:
        RuntimeAdminClient(None)  # type: ignore[arg-type]
    except SDKError as exc:
        assert "daemon control is required" in exc.message
    else:
        raise AssertionError("constructor accepted missing control")
