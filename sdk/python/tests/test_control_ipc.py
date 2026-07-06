import json
import socket
import struct
import tempfile
import threading
import unittest
from pathlib import Path

from easynet_sdk import (
    CONTROL_IPC_VERSION,
    ControlDiscovery,
    ControlIpcClient,
    ErrorCode,
    IpcVersionRange,
    SDKError,
    SdkEnvironment,
    is_code,
    read_control_discovery,
)


class ControlIpcTests(unittest.TestCase):
    def test_reads_discovery_and_negotiates_v1(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "control.json"
            path.write_text(
                json.dumps(
                    {
                        "socket_path": f"{tmp}/control.sock",
                        "invocation_endpoint": f"{tmp}/daemon.sock",
                        "pid": 123,
                        "daemon_version": "test",
                        "supported_ipc_versions": {"min": 1, "max": 1},
                        "capability_flags": ["boot_status"],
                    }
                )
            )

            discovery = read_control_discovery(path)

        self.assertEqual(discovery.socket_path, f"{tmp}/control.sock")
        self.assertEqual(discovery.invocation_endpoint, f"{tmp}/daemon.sock")
        self.assertEqual(discovery.supported_ipc_versions, IpcVersionRange(1, 1))
        self.assertEqual(discovery.capability_flags, ("boot_status",))

    def test_missing_discovery_maps_to_daemon_offline(self) -> None:
        with self.assertRaises(SDKError) as caught:
            read_control_discovery("/tmp/no-such-control-file-for-sdk-test.json")

        self.assertTrue(is_code(caught.exception, ErrorCode.DAEMON_OFFLINE))

    def test_disjoint_version_range_fails_before_dial(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "control.json"
            path.write_text(
                json.dumps(
                    {
                        "socket_path": f"{tmp}/control.sock",
                        "pid": 123,
                        "daemon_version": "test",
                        "supported_ipc_versions": {"min": 99, "max": 100},
                    }
                )
            )

            with self.assertRaises(SDKError) as caught:
                ControlIpcClient.connect(path)

        self.assertTrue(is_code(caught.exception, ErrorCode.VERSION_MISMATCH))

    def test_round_trip_uses_little_endian_length_prefixed_json(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            sock_path = Path(tmp) / "control.sock"
            control_path = Path(tmp) / "control.json"
            control_path.write_text(
                json.dumps(
                    {
                        "socket_path": str(sock_path),
                        "invocation_endpoint": f"{tmp}/daemon.sock",
                        "pid": 123,
                        "daemon_version": "test",
                        "supported_ipc_versions": {"min": 1, "max": 1},
                        "capability_flags": ["boot_status"],
                    }
                )
            )
            server = _OneShotControlServer(sock_path)
            server.start()
            try:
                client = ControlIpcClient.connect(control_path, timeout=1.0)
                frame = client.round_trip(
                    {
                        "type": "subscribe",
                        "subscription_id": "boot-sub",
                        "ability": "system.watch_boot",
                        "args": {},
                    }
                )
                client.close()
            finally:
                server.close()

        self.assertEqual(client.ipc_version, CONTROL_IPC_VERSION)
        self.assertEqual(frame.frame_type, "frame")
        self.assertEqual(frame.subscription_id, "boot-sub")
        self.assertEqual(frame.frame, {"type": "ready"})
        self.assertEqual(
            server.received,
            {
                "ability": "system.watch_boot",
                "args": {},
                "subscription_id": "boot-sub",
                "type": "subscribe",
            },
        )

    def test_rejects_product_ability_subscribe_before_socket_write(self) -> None:
        first, second = socket.socketpair()
        try:
            discovery = ControlDiscovery(
                socket_path="/tmp/control.sock",
                supported_ipc_versions=IpcVersionRange(1, 1),
            )
            client = ControlIpcClient(first, discovery=discovery, ipc_version=1)

            with self.assertRaises(SDKError) as caught:
                client.subscribe("product-sub", "directory.subscribe")

            self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
            self.assertIn("system.watch_boot", caught.exception.message)
            second.settimeout(0.05)
            with self.assertRaises(socket.timeout):
                second.recv(1)
        finally:
            first.close()
            second.close()

    def test_generic_send_rejects_product_dispatch_frame(self) -> None:
        first, second = socket.socketpair()
        try:
            discovery = ControlDiscovery(
                socket_path="/tmp/control.sock",
                supported_ipc_versions=IpcVersionRange(1, 1),
            )
            client = ControlIpcClient(first, discovery=discovery, ipc_version=1)

            for frame in (
                {"type": "invoke", "ability": "observe.health", "args": {}},
                {"type": "OpenBidi", "ability": "terminal.attach", "args": {}},
                {
                    "type": "subscribe",
                    "subscription_id": "x",
                    "ability": "events.device.subscribe",
                },
            ):
                with self.subTest(frame=frame):
                    with self.assertRaises(SDKError) as caught:
                        client.send(frame)
                    self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

            second.settimeout(0.05)
            with self.assertRaises(socket.timeout):
                second.recv(1)
        finally:
            first.close()
            second.close()

    def test_recv_rejects_oversized_frame_before_allocation(self) -> None:
        first, second = socket.socketpair()
        try:
            discovery = ControlDiscovery(
                socket_path="/tmp/control.sock",
                supported_ipc_versions=IpcVersionRange(1, 1),
            )
            client = ControlIpcClient(
                first,
                discovery=discovery,
                ipc_version=1,
                max_frame_bytes=8,
            )
            second.sendall(struct.pack("<I", 9))

            with self.assertRaises(SDKError) as caught:
                client.recv()

            self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        finally:
            first.close()
            second.close()

    def test_environment_opens_and_owns_control_ipc_client(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            sock_path = Path(tmp) / "control.sock"
            control_path = Path(tmp) / "control.json"
            control_path.write_text(
                json.dumps(
                    {
                        "socket_path": str(sock_path),
                        "pid": 123,
                        "daemon_version": "test",
                        "supported_ipc_versions": {"min": 1, "max": 1},
                    }
                )
            )
            server = _OneShotControlServer(sock_path)
            server.start()
            try:
                env = SdkEnvironment(control_path=str(control_path))
                client = env.control_ipc_client(timeout=1.0)
                client.subscribe("boot-sub", "system.watch_boot")
                frame = client.recv()
                env.close()
            finally:
                server.close()

        self.assertEqual(frame.frame_type, "frame")
        assert server.received is not None
        self.assertEqual(server.received["type"], "subscribe")
        with self.assertRaises(SDKError) as caught:
            client.recv()
        self.assertTrue(is_code(caught.exception, ErrorCode.CANCELLED))

    def test_use_after_close_is_cancelled(self) -> None:
        first, second = socket.socketpair()
        try:
            discovery = ControlDiscovery(
                socket_path="/tmp/control.sock",
                supported_ipc_versions=IpcVersionRange(1, 1),
            )
            client = ControlIpcClient(first, discovery=discovery, ipc_version=1)
            client.close()

            with self.assertRaises(SDKError) as caught:
                client.send({"type": "cancel", "subscription_id": "boot-sub"})

            self.assertTrue(is_code(caught.exception, ErrorCode.CANCELLED))
        finally:
            first.close()
            second.close()


class _OneShotControlServer:
    def __init__(self, path: Path) -> None:
        self.path = path
        self.received: dict[str, object] | None = None
        self._listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._listener.bind(str(path))
        self._listener.listen(1)
        self._thread = threading.Thread(target=self._run, daemon=True)

    def start(self) -> None:
        self._thread.start()

    def close(self) -> None:
        self._listener.close()
        self._thread.join(timeout=1.0)

    def _run(self) -> None:
        conn, _ = self._listener.accept()
        with conn:
            size = struct.unpack("<I", _recv_exact(conn, 4))[0]
            self.received = json.loads(_recv_exact(conn, size).decode("utf-8"))
            raw = json.dumps(
                {
                    "type": "frame",
                    "subscription_id": self.received["subscription_id"],
                    "frame": {"type": "ready"},
                },
                separators=(",", ":"),
                sort_keys=True,
            ).encode("utf-8")
            conn.sendall(struct.pack("<I", len(raw)) + raw)


def _recv_exact(conn: socket.socket, size: int) -> bytes:
    chunks: list[bytes] = []
    remaining = size
    while remaining:
        chunk = conn.recv(remaining)
        if not chunk:
            raise RuntimeError("socket closed")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


if __name__ == "__main__":
    unittest.main()
