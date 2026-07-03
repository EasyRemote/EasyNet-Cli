import json
import unittest

from easynet_sdk import (
    AttachOptions,
    ConnectOptions,
    DaemonLifecycleState,
    DaemonMode,
    DiscoverOptions,
    ErrorCode,
    SDKError,
    StartConfig,
    StopOptions,
    attach_daemon,
    discover_daemon,
    is_code,
    start_daemon,
)


class MemoryRuntimeTransport:
    def invoke(self, draft_json: bytes) -> bytes:
        raise RuntimeError("not used")


class MemoryDaemonTransport:
    def __init__(self) -> None:
        self.discover_json = (
            b'{"control_endpoint":"unix:///tmp/control.sock",'
            b'"invocation_endpoint":"unix:///tmp/daemon.sock"}'
        )
        self.start_json = ready_status()
        self.attach_json = ready_status()
        self.status_json = ready_status()
        self.stop_json = b'{"handle_id":"daemon-1","state":"Stopped","mode":"hub"}'
        self.start_calls = 0
        self.stop_calls = 0
        self.detach_calls = 0
        self.open_calls = 0
        self.seen_start: dict[str, object] | None = None
        self.seen_options: dict[str, object] | None = None

    def discover(self, options_json: bytes) -> bytes:
        self.seen_options = json.loads(options_json.decode("utf-8"))
        return self.discover_json

    def start(self, config_json: bytes) -> bytes:
        self.start_calls += 1
        self.seen_start = json.loads(config_json.decode("utf-8"))
        return self.start_json

    def attach(self, options_json: bytes) -> bytes:
        self.seen_options = json.loads(options_json.decode("utf-8"))
        return self.attach_json

    def status(self, handle_id: str) -> bytes:
        return self.status_json

    def open_runtime(self, handle_id: str, options_json: bytes):
        self.open_calls += 1
        self.seen_options = json.loads(options_json.decode("utf-8"))
        return MemoryRuntimeTransport(), b'{"ready":true}'

    def stop(self, handle_id: str, options_json: bytes) -> bytes:
        self.stop_calls += 1
        return self.stop_json

    def detach(self, handle_id: str) -> None:
        self.detach_calls += 1


def ready_status() -> bytes:
    return (
        b'{"handle_id":"daemon-1","state":"Running","mode":"hub","pid":42,'
        b'"endpoints":{"control_endpoint":"unix:///tmp/control.sock",'
        b'"invocation_endpoint":"unix:///tmp/daemon.sock",'
        b'"public_endpoint":"https://hub.example"}}'
    )


class DaemonTests(unittest.TestCase):
    def test_start_returns_runtime_ready_handle(self) -> None:
        transport = MemoryDaemonTransport()

        handle = start_daemon(
            transport,
            StartConfig(
                mode=DaemonMode.HUB,
                listen_tcp="127.0.0.1:9443",
                tls_cert_path="/tmp/cert.pem",
                tls_key_path="/tmp/key.pem",
            ),
        )

        self.assertEqual(handle.handle_id, "daemon-1")
        self.assertEqual(handle.state, DaemonLifecycleState.RUNNING)
        assert transport.seen_start is not None
        self.assertEqual(transport.seen_start["listen_tcp"], "127.0.0.1:9443")
        self.assertEqual(handle.endpoints.invocation_endpoint, "unix:///tmp/daemon.sock")

    def test_start_rejects_unsafe_mode_policy_before_transport(self) -> None:
        transport = MemoryDaemonTransport()

        with self.assertRaises(SDKError):
            start_daemon(
                transport,
                StartConfig(mode=DaemonMode.DEVICE, listen_tcp="0.0.0.0:9443"),
            )
        self.assertEqual(transport.start_calls, 0)

        with self.assertRaises(SDKError):
            start_daemon(
                transport,
                StartConfig(mode=DaemonMode.HUB, listen_tcp="0.0.0.0:9443"),
            )
        self.assertEqual(transport.start_calls, 0)

    def test_attach_rejects_control_only_readiness(self) -> None:
        transport = MemoryDaemonTransport()
        transport.attach_json = (
            b'{"handle_id":"daemon-1","state":"ControlOnly",'
            b'"endpoints":{"control_endpoint":"unix:///tmp/control.sock"}}'
        )

        with self.assertRaises(SDKError) as caught:
            attach_daemon(
                transport,
                AttachOptions(control_endpoint="unix:///tmp/control.sock"),
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.CONTROL_ONLY))

    def test_discover_preserves_advertised_invocation_endpoint(self) -> None:
        transport = MemoryDaemonTransport()
        transport.discover_json = (
            b'{"control_endpoint":"unix:///tmp/control.sock",'
            b'"invocation_endpoint":"unix:///tmp/custom-daemon.sock"}'
        )

        endpoints = discover_daemon(transport, DiscoverOptions(home_dir="/tmp/home"))

        self.assertEqual(
            endpoints.invocation_endpoint,
            "unix:///tmp/custom-daemon.sock",
        )

    def test_open_runtime_requires_ready_state(self) -> None:
        transport = MemoryDaemonTransport()
        handle = start_daemon(transport, StartConfig(mode=DaemonMode.HUB))

        client = handle.open_runtime(ConnectOptions(max_message_bytes=4096))

        self.assertIsNotNone(client)
        self.assertEqual(transport.open_calls, 1)
        assert transport.seen_options is not None
        self.assertEqual(transport.seen_options["max_message_bytes"], 4096)

        handle._status = handle._status.__class__(
            state=DaemonLifecycleState.CONTROL_READY,
            handle_id=handle.handle_id,
        )
        with self.assertRaises(SDKError):
            handle.open_runtime()

    def test_stop_is_idempotent_and_detach_does_not_stop(self) -> None:
        transport = MemoryDaemonTransport()
        handle = start_daemon(transport, StartConfig(mode=DaemonMode.HUB))

        handle.stop(StopOptions())
        handle.stop(StopOptions())
        self.assertEqual(transport.stop_calls, 1)

        handle.detach()
        self.assertEqual(transport.detach_calls, 1)
        self.assertEqual(transport.stop_calls, 1)
        with self.assertRaises(SDKError) as caught:
            handle.status()
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_HANDLE))


if __name__ == "__main__":
    unittest.main()
