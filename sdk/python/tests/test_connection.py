import json
import unittest

from easynet_sdk import (
    ConnectOptions,
    ConnectionState,
    ControlDiscovery,
    ControlDiscoveryRuntimeConnector,
    ErrorCode,
    IpcVersionRange,
    RuntimeConnection,
    SDKError,
    is_code,
)


class MemoryRuntimeConnector:
    def __init__(self) -> None:
        self.seen_options: dict[str, object] | None = None
        self.closed = False
        self.resolve_error: BaseException | None = None
        self.handshake_error: BaseException | None = None
        self.handshake_endpoint: dict[str, object] | None = None

    def resolve(self, options_json: bytes) -> bytes:
        if self.resolve_error is not None:
            raise self.resolve_error
        self.seen_options = json.loads(options_json.decode("utf-8"))
        return (
            b'{"endpoint":"unix:///tmp/daemon.sock",'
            b'"control_path":"/tmp/control.sock",'
            b'"protocol_version":"v4","abi_version":4}'
        )

    def handshake(self, endpoint_json: bytes):
        if self.handshake_error is not None:
            raise self.handshake_error
        self.handshake_endpoint = json.loads(endpoint_json.decode("utf-8"))
        return MemoryRuntimeTransport(), b'{"ready":true}'

    def close(self) -> None:
        self.closed = True


class MemoryRuntimeTransport:
    def invoke(self, draft_json: bytes) -> bytes:
        return b"{}"

    def prepare(self, draft_json: bytes, options_json: bytes) -> bytes:
        return b"{}"

    def submit_signed(self, signed_json: bytes) -> bytes:
        return b"{}"

    def await_handle(self, handle_id: int) -> bytes:
        return b"{}"

    def cancel_handle(self, handle_id: int, reason: str) -> bytes:
        return b"{}"

    def handle_events(self, handle_id: int) -> bytes:
        return b"{}"


class RuntimeConnectionTests(unittest.TestCase):
    def test_connect_reaches_ready_and_returns_runtime_client(self) -> None:
        connector = MemoryRuntimeConnector()
        connection = RuntimeConnection(connector)

        connection.connect(
            ConnectOptions(endpoint="unix:///tmp/daemon.sock", max_message_bytes=4096)
        )

        self.assertEqual(connection.state, ConnectionState.READY)
        assert connection.endpoint is not None
        self.assertEqual(connection.endpoint.endpoint, "unix:///tmp/daemon.sock")
        self.assertEqual(connector.seen_options, {
            "endpoint": "unix:///tmp/daemon.sock",
            "max_message_bytes": 4096,
        })
        self.assertIsNotNone(connection.runtime_client())

    def test_connect_failure_blocks_runtime_client(self) -> None:
        connector = MemoryRuntimeConnector()
        connector.resolve_error = RuntimeError("daemon down")
        connection = RuntimeConnection(connector)

        with self.assertRaises(SDKError) as caught:
            connection.connect()

        self.assertTrue(is_code(caught.exception, ErrorCode.TRANSPORT))
        self.assertEqual(connection.state, ConnectionState.FAILED)
        with self.assertRaises(SDKError):
            connection.runtime_client()

    def test_close_is_terminal(self) -> None:
        connector = MemoryRuntimeConnector()
        connection = RuntimeConnection(connector)
        connection.connect()

        connection.close()

        self.assertTrue(connector.closed)
        self.assertEqual(connection.state, ConnectionState.CLOSED)
        with self.assertRaises(SDKError):
            connection.runtime_client()
        with self.assertRaises(SDKError):
            connection.connect()


class ControlDiscoveryRuntimeConnectorTests(unittest.TestCase):
    def test_resolve_uses_explicit_endpoint_without_reading_discovery(self) -> None:
        inner = MemoryRuntimeConnector()
        connector = ControlDiscoveryRuntimeConnector(
            inner,
            control_path="/tmp/default-control.json",
            discovery_reader=self._raising_reader,
        )

        raw = connector.resolve(
            ConnectOptions(
                endpoint="unix:///tmp/runtime.sock",
                control_path="/tmp/custom-control.json",
            ).to_json_bytes()
        )

        endpoint = json.loads(raw.decode("utf-8"))
        self.assertEqual(endpoint["endpoint"], "unix:///tmp/runtime.sock")
        self.assertEqual(endpoint["control_path"], "/tmp/custom-control.json")

    def test_resolve_reads_invocation_endpoint_from_control_discovery(self) -> None:
        inner = MemoryRuntimeConnector()
        connector = ControlDiscoveryRuntimeConnector(
            inner,
            control_path="/tmp/default-control.json",
            discovery_reader=self._ready_discovery,
        )

        raw = connector.resolve(ConnectOptions().to_json_bytes())

        endpoint = json.loads(raw.decode("utf-8"))
        self.assertEqual(endpoint["endpoint"], "unix:///tmp/invocation.sock")
        self.assertEqual(endpoint["control_path"], "/tmp/default-control.json")
        self.assertEqual(endpoint["control_endpoint"], "/tmp/control.sock")
        self.assertEqual(endpoint["daemon_version"], "1.2.3")
        self.assertEqual(endpoint["capability_flags"], ["runtime"])

    def test_resolve_control_only_when_discovery_has_no_invocation_endpoint(self) -> None:
        connector = ControlDiscoveryRuntimeConnector(
            MemoryRuntimeConnector(),
            discovery_reader=self._control_only_discovery,
        )

        with self.assertRaises(SDKError) as caught:
            connector.resolve(
                ConnectOptions(control_path="/tmp/control.json").to_json_bytes()
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.CONTROL_ONLY))
        self.assertEqual(caught.exception.stage, "connection")
        self.assertEqual(caught.exception.details["control_path"], "/tmp/control.json")

    def test_connection_delegates_handshake_to_inner_connector(self) -> None:
        inner = MemoryRuntimeConnector()
        connection = RuntimeConnection(
            ControlDiscoveryRuntimeConnector(
                inner,
                discovery_reader=self._ready_discovery,
            )
        )

        connection.connect(ConnectOptions(control_path="/tmp/control.json"))

        self.assertEqual(connection.state, ConnectionState.READY)
        assert connection.endpoint is not None
        self.assertEqual(connection.endpoint.endpoint, "unix:///tmp/invocation.sock")
        self.assertEqual(connection.endpoint.control_endpoint, "/tmp/control.sock")
        self.assertEqual(connection.endpoint.daemon_version, "1.2.3")
        self.assertEqual(connection.endpoint.capability_flags, ("runtime",))
        self.assertEqual(
            inner.handshake_endpoint["endpoint"], "unix:///tmp/invocation.sock"
        )

    def test_close_delegates_to_inner_connector_once(self) -> None:
        inner = MemoryRuntimeConnector()
        connector = ControlDiscoveryRuntimeConnector(inner)

        connector.close()
        connector.close()

        self.assertTrue(inner.closed)

    @staticmethod
    def _ready_discovery(control_path: str) -> ControlDiscovery:
        return ControlDiscovery(
            socket_path="/tmp/control.sock",
            invocation_endpoint="unix:///tmp/invocation.sock",
            daemon_version="1.2.3",
            supported_ipc_versions=IpcVersionRange(1, 1),
            capability_flags=("runtime",),
        )

    @staticmethod
    def _control_only_discovery(control_path: str) -> ControlDiscovery:
        return ControlDiscovery(
            socket_path="/tmp/control.sock",
            supported_ipc_versions=IpcVersionRange(1, 1),
        )

    @staticmethod
    def _raising_reader(control_path: str) -> ControlDiscovery:
        raise AssertionError("discovery must not be read for explicit endpoint")


if __name__ == "__main__":
    unittest.main()
