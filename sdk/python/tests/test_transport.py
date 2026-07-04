import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from easynet_sdk import (
    BidiState,
    DaemonInvocationTransport,
    ErrorCode,
    RuntimeClient,
    SDKError,
    StreamState,
    is_code,
)
from easynet_sdk._cabi import CLILibrary

from test_cabi import FakeRawCABI
from test_runtime import MemoryRuntimeTransport, complete_draft


def _load_patch(raw: FakeRawCABI):
    return patch("easynet_sdk._cabi.CLILibrary.load", return_value=CLILibrary(raw))


class DaemonInvocationTransportTests(unittest.TestCase):
    def test_invoke_accepts_complete_invocation_mapping(self) -> None:
        runtime = MemoryRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(runtime))

        result = transport.invoke(complete_draft().to_json_dict())

        self.assertTrue(result["ok"])
        self.assertEqual(result["output_json"], {"ready": True})
        self.assertEqual(result["terminal_state"], "Completed")
        assert runtime.seen_draft is not None
        self.assertEqual(
            runtime.seen_draft["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )

    def test_invoke_projects_runtime_receipt_summary_to_dict(self) -> None:
        class ReceiptRuntimeTransport(MemoryRuntimeTransport):
            def invoke(self, draft_json: bytes) -> bytes:
                result = json.loads(super().invoke(draft_json).decode("utf-8"))
                result["receipt"] = {
                    "receipt_ura": "easynet:///r/example/receipt/opaque",
                    "invocation_id": "inv-1",
                    "state": "completed",
                    "self_hash_hex": "00" * 32,
                    "cleanup_complete": True,
                }
                return json.dumps(
                    result, separators=(",", ":"), sort_keys=True
                ).encode("utf-8")

        runtime = ReceiptRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(runtime))

        result = transport.invoke(complete_draft().to_json_dict())

        self.assertEqual(result["receipt"]["invocation_id"], "inv-1")
        self.assertEqual(result["receipt_summary"]["invocation_id"], "inv-1")
        self.assertTrue(result["receipt_summary"]["has_causal_anchor"])

    def test_connect_owns_runtime_connection_lifecycle(self) -> None:
        raw = FakeRawCABI()
        with tempfile.TemporaryDirectory() as tmp:
            control_path = _write_control_discovery(tmp)
            with _load_patch(raw):
                transport = DaemonInvocationTransport.connect(
                    control_path=str(control_path)
                )
                self.assertIsNotNone(transport.connection)
                result = transport.invoke(complete_draft().to_json_dict())
                transport.close()
                transport.close()

        self.assertTrue(result["ok"])
        self.assertEqual(raw.daemon_discovers, [])
        self.assertEqual(raw.daemon_open_clients, [707])
        self.assertEqual(raw.daemon_detaches, [707])
        self.assertEqual(raw.shutdown_handles, [808])

    def test_connect_rejects_control_only_discovery(self) -> None:
        raw = FakeRawCABI()
        with tempfile.TemporaryDirectory() as tmp:
            control_path = _write_control_discovery(tmp, invocation_endpoint="")
            with _load_patch(raw):
                with self.assertRaises(SDKError) as caught:
                    DaemonInvocationTransport.connect(control_path=str(control_path))

        self.assertTrue(is_code(caught.exception, ErrorCode.CONTROL_ONLY))
        self.assertEqual(raw.daemon_attaches, [])
        self.assertEqual(raw.daemon_open_clients, [])

    def test_stream_projects_sdk_events_to_dicts(self) -> None:
        runtime = MemoryRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(runtime))

        stream = transport.stream(complete_draft())
        event = stream.recv()
        stream.close()

        self.assertTrue(event["terminal"])
        self.assertEqual(event["kind"], "terminal")

    def test_stream_timeout_is_forwarded_without_state_mutation(self) -> None:
        class TimeoutRuntimeTransport(MemoryRuntimeTransport):
            def open_stream(self, draft_json: bytes):
                from test_stream import MemoryStreamTransport

                class TimeoutStreamTransport(MemoryStreamTransport):
                    def recv(self, timeout: float | None = None) -> bytes:
                        self.timeout = timeout
                        raise TimeoutError("no frame")

                self.stream_transport = TimeoutStreamTransport()
                return (
                    self.stream_transport,
                    b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
                )

        runtime = TimeoutRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(runtime))

        stream = transport.stream(complete_draft())
        with self.assertRaises(TimeoutError):
            stream.recv(timeout=0.01)

        self.assertEqual(runtime.stream_transport.timeout, 0.01)
        self.assertEqual(stream.handle.state, StreamState.OPEN)

    def test_bidi_keeps_half_close_cancel_and_close_distinct(self) -> None:
        runtime = MemoryRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(runtime))
        channel = transport.bidi(
            complete_draft().to_json_dict(),
            [{"stream_id": 1, "content_type": "application/json"}],
        )

        ack = channel.send({"sequence": 1, "kind": "data", "stream_id": 1})
        half_closed = channel.close_send()
        with self.assertRaises(SDKError) as caught:
            channel.close()
        cancelled = channel.cancel("client stop")
        channel.close()

        self.assertEqual(ack["sequence"], 1)
        self.assertEqual(half_closed["state"], "HalfClosedLocal")
        self.assertFalse(half_closed["terminal"])
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(cancelled["state"], "Cancelled")
        self.assertEqual(channel.session.state, BidiState.CLOSED)

    def test_bidi_timeout_is_forwarded_without_state_mutation(self) -> None:
        class TimeoutRuntimeTransport(MemoryRuntimeTransport):
            def open_bidi(self, draft_json: bytes, streams_json: bytes):
                from test_bidi import MemoryBidiTransport

                class TimeoutBidiTransport(MemoryBidiTransport):
                    def recv(self, timeout: float | None = None) -> bytes:
                        self.timeout = timeout
                        raise TimeoutError("no frame")

                self.bidi_transport = TimeoutBidiTransport()
                return (
                    self.bidi_transport,
                    b'{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}',
                )

        runtime = TimeoutRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(runtime))

        channel = transport.bidi(
            complete_draft().to_json_dict(),
            [{"stream_id": 1, "content_type": "application/json"}],
        )
        with self.assertRaises(TimeoutError):
            channel.recv(timeout=0.01)

        self.assertEqual(runtime.bidi_transport.timeout, 0.01)
        self.assertEqual(channel.session.state, BidiState.OPEN)

    def test_rejects_incomplete_invocation_mapping_before_dispatch(self) -> None:
        runtime = MemoryRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(runtime))

        with self.assertRaises(SDKError) as caught:
            transport.invoke({"caller_ura": "easynet:///r/example/agent/alice.sdk"})

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(runtime.seen_draft)

def _write_control_discovery(tmp: str, *, invocation_endpoint: str | None = None) -> Path:
    path = Path(tmp) / "control.json"
    value = {
        "socket_path": f"{tmp}/control.sock",
        "pid": 123,
        "daemon_version": "1.2.3",
        "supported_ipc_versions": {"min": 1, "max": 1},
        "capability_flags": ["runtime"],
    }
    if invocation_endpoint is None:
        value["invocation_endpoint"] = f"{tmp}/daemon.sock"
    elif invocation_endpoint:
        value["invocation_endpoint"] = invocation_endpoint
    path.write_text(
        json.dumps(value, separators=(",", ":"), sort_keys=True),
        encoding="utf-8",
    )
    return path


if __name__ == "__main__":
    unittest.main()
