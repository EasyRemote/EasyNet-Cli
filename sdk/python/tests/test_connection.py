import json
import unittest

from easynet_sdk import (
    ConnectOptions,
    ConnectionState,
    ErrorCode,
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


if __name__ == "__main__":
    unittest.main()

