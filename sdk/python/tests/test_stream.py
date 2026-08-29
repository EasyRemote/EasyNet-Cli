import threading
import unittest
from collections.abc import Callable

from easynet_sdk import (
    ErrorCode,
    RetryHint,
    SDKError,
    StreamCancel,
    StreamEvent,
    StreamHandle,
    StreamState,
    StreamTerminalEvent,
    is_code,
)
from easynet_sdk.stream import RawStreamPacket


class MemoryStreamTransport:
    def __init__(self, events: list[bytes] | None = None) -> None:
        self.events = list(events or [])
        self.closed = False
        self.cancel_reason = ""
        self.cancel_reply = (
            b'{"stream_id":"stream-1","cancelled":false,'
            b'"state":"CancelRequested","terminal":false}'
        )

    def recv(self, timeout: float | None = None) -> bytes:
        if not self.events:
            raise RuntimeError("no event")
        return self.events.pop(0)

    def cancel(self, reason: str) -> bytes:
        self.cancel_reason = reason
        return self.cancel_reply

    def close(self) -> None:
        self.closed = True


class UnsupportedCancelStreamTransport(MemoryStreamTransport):
    def cancel(self, reason: str) -> bytes:
        del reason
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="stream cancellation unsupported",
        )


class InterruptedCancelStreamTransport(MemoryStreamTransport):
    def cancel(self, reason: str) -> bytes:
        del reason
        raise TimeoutError("cancel request deadline elapsed")


class ConcurrentCancelStreamTransport(MemoryStreamTransport):
    def __init__(self) -> None:
        super().__init__()
        self.recv_started = threading.Event()
        self.terminal_ready = threading.Event()

    def recv(self, timeout: float | None = None) -> bytes:
        self.recv_started.set()
        if not self.terminal_ready.wait(timeout=timeout or 1.0):
            raise TimeoutError("terminal frame was not released")
        return (
            b'{"sequence":1,"kind":"terminal","state":"Cancelled",'
            b'"terminal":true,"terminal_receipt":{"receipt_id":"cancelled-1"}}'
        )

    def cancel(self, reason: str) -> bytes:
        self.cancel_reason = reason
        self.terminal_ready.set()
        return self.cancel_reply


def _capture_result(
    operation: Callable[[], StreamEvent],
    results: list[StreamEvent],
    errors: list[BaseException],
) -> None:
    try:
        results.append(operation())
    except BaseException as exc:
        errors.append(exc)


class StreamTests(unittest.TestCase):
    def test_stream_timeout_does_not_change_state(self) -> None:
        class TimeoutStreamTransport(MemoryStreamTransport):
            def recv(self, timeout: float | None = None) -> bytes:
                self.timeout = timeout
                raise TimeoutError("no frame")

        transport = TimeoutStreamTransport()
        stream = StreamHandle.from_json(
            transport,
            b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
        )

        with self.assertRaises(TimeoutError):
            stream.next(timeout=0.01)

        self.assertEqual(transport.timeout, 0.01)
        self.assertEqual(stream.state, StreamState.OPEN)

    def test_stream_orders_events_and_closes_after_terminal(self) -> None:
        transport = MemoryStreamTransport(
            [
                b'{"sequence":1,"kind":"data","state":"Open","terminal":false,'
                b'"payload_content_type":"application/json","payload_json":{"step":1}}',
                b'{"sequence":2,"kind":"terminal","state":"Completed","terminal":true,'
                b'"payload_content_type":"application/json","payload_json":{"ok":true}}',
            ]
        )
        stream = StreamHandle.from_json(
            transport,
            b'{"stream_id":"stream-1","state":"Opening","max_buffered_events":4}',
        )

        first = stream.next()
        terminal = stream.next()
        stream.close()

        self.assertEqual(first.sequence, 1)
        self.assertTrue(terminal.terminal)
        terminal_projection = stream.terminal_event()
        self.assertIsInstance(terminal_projection, StreamTerminalEvent)
        self.assertEqual(terminal_projection.stream_id, "stream-1")
        self.assertEqual(terminal_projection.event_type, "terminal")
        self.assertEqual(terminal_projection.seq, 2)
        self.assertEqual(terminal_projection.payload, {"ok": True})
        self.assertTrue(transport.closed)
        self.assertEqual(stream.state, StreamState.CLOSED)
        self.assertEqual(stream.runtime_state, StreamState.TERMINAL_FRAME_SEEN)

    def test_stream_event_rejects_legacy_content_type_alias(self) -> None:
        with self.assertRaises(SDKError) as caught:
            StreamEvent.from_json(
                b'{"sequence":1,"kind":"data","content_type":"application/json"}'
            )

        self.assertIn(
            "stream event contains noncanonical field content_type",
            str(caught.exception),
        )

    def test_stream_projections_reject_product_state_code(self) -> None:
        with self.assertRaises(SDKError) as open_caught:
            StreamHandle.from_json(
                MemoryStreamTransport(),
                b'{"stream_id":"stream-1","state":"Open",'
                b'"max_buffered_events":4,"state_code":"S200"}',
            )
        self.assertIn(
            "stream open contains noncanonical field state_code",
            str(open_caught.exception),
        )
        with self.assertRaises(SDKError) as event_caught:
            StreamEvent.from_json(
                b'{"sequence":1,"kind":"data","state":"Open",'
                b'"terminal":false,"state_code":"S200"}'
            )
        self.assertIn(
            "stream event contains noncanonical field state_code",
            str(event_caught.exception),
        )
        with self.assertRaises(SDKError) as cancel_caught:
            StreamCancel.from_json(
                b'{"stream_id":"stream-1","cancelled":false,'
                b'"state":"CancelRequested","terminal":false,'
                b'"state_code":"S200"}'
            )
        self.assertIn(
            "stream cancel contains noncanonical field state_code",
            str(cancel_caught.exception),
        )

    def test_stream_event_rejects_legacy_chunk_kind(self) -> None:
        with self.assertRaises(SDKError) as ctx:
            StreamEvent.from_json(
                b'{"sequence":1,"kind":"chunk","state":"Open","terminal":false}'
            )

        self.assertEqual(ctx.exception.code, ErrorCode.INVALID_ARGUMENT)

    def test_binary_stream_packet_requires_positive_sequence(self) -> None:
        with self.assertRaises(SDKError) as caught:
            StreamEvent.from_raw_packet(
                RawStreamPacket(
                    sequence=0,
                    kind="data",
                    state="Running",
                    terminal=False,
                    transport_terminal=False,
                    elapsed_ms=0,
                    payload_content_type="video/h264",
                    payload=b"\x00\x01",
                )
            )

        self.assertIn("sequence must be positive", str(caught.exception))

    def test_raw_stream_packet_projects_raw_payload_without_payload_aliases(self) -> None:
        event = StreamEvent.from_raw_packet(
            RawStreamPacket(
                sequence=1,
                kind="data",
                state="Running",
                terminal=False,
                transport_terminal=False,
                elapsed_ms=0,
                payload_content_type="video/h264",
                payload=b"\x00\x01\x02",
            )
        )

        self.assertEqual(event.payload_bytes, b"\x00\x01\x02")
        self.assertEqual(event.payload_base64, "")
        self.assertIsNone(event.payload_json)
        self.assertFalse(event.terminal)
        self.assertFalse(event.transport_terminal)

    def test_binary_stream_packet_rejects_non_object_sidecar(self) -> None:
        with self.assertRaises(SDKError) as caught:
            StreamEvent.from_raw_packet(
                RawStreamPacket(
                    sequence=1,
                    kind="data",
                    state="Running",
                    terminal=False,
                    transport_terminal=False,
                    elapsed_ms=0,
                    payload_content_type="video/h264",
                    payload=b"\x00\x01",
                    error_json=b"[]",
                )
            )

        self.assertIn("error sidecar must be an object", str(caught.exception))

    def test_raw_stream_packet_rejects_noncanonical_state_and_object_fields(self) -> None:
        for field_name, value in (("state", "not-a-runtime-state"), ("kind", "chunk")):
            with self.subTest(field=field_name):
                fields = {
                    "sequence": 1,
                    "kind": "data",
                    "state": "Running",
                    "terminal": False,
                    "transport_terminal": False,
                    "elapsed_ms": 0,
                    "payload_content_type": "video/h264",
                    "payload": b"\x00",
                }
                fields[field_name] = value
                with self.assertRaises(SDKError):
                    StreamEvent.from_raw_packet(RawStreamPacket(**fields))

    def test_stream_terminal_event_projects_terminal_receipt(self) -> None:
        transport = MemoryStreamTransport(
            [
                b'{"sequence":1,"kind":"terminal","state":"Completed",'
                b'"terminal":true,"payload_json":{"ok":true},'
                b'"elapsed_ms":12,"terminal_receipt":{"receipt_ura":'
                b'"easynet:///r/example/resource/agent.alice.sdk/invocation/r1/receipt"}}'
            ]
        )
        stream = StreamHandle.from_json(
            transport,
            b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
        )

        event = stream.next()
        terminal = stream.terminal_event()

        self.assertEqual(event.elapsed_ms, 12)
        self.assertEqual(
            terminal.terminal_receipt,
            {
                "receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/r1/receipt"
            },
        )
        self.assertIn(b'"event_type":"terminal"', terminal.to_json())
        self.assertIn(b'"terminal_receipt":', terminal.to_json())
        self.assertNotIn(b'"receipt":', terminal.to_json())

    def test_stream_event_rejects_legacy_receipt_only_field(self) -> None:
        with self.assertRaises(SDKError) as caught:
            StreamEvent.from_json(
                b'{"sequence":1,"kind":"terminal","state":"Completed",'
                b'"terminal":true,"receipt":{"receipt_id":"legacy-only"}}'
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_stream_event_rejects_legacy_event_alias(self) -> None:
        with self.assertRaises(SDKError) as caught:
            StreamEvent.from_json(
                b'{"sequence":1,"event":"chunk","state":"Open","terminal":false}'
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_stream_rejects_next_after_terminal(self) -> None:
        transport = MemoryStreamTransport(
            [
                b'{"sequence":1,"kind":"terminal","state":"Completed","terminal":true}',
                b'{"sequence":2,"kind":"terminal","state":"Completed","terminal":true}',
            ]
        )
        stream = StreamHandle.from_json(
            transport,
            b'{"stream_id":"stream-1","state":"Opening","max_buffered_events":4}',
        )

        stream.next()
        with self.assertRaises(SDKError) as caught:
            stream.next()

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(stream.state, StreamState.TERMINAL_FRAME_SEEN)

    def test_stream_rejects_duplicate_runtime_sequence(self) -> None:
        transport = MemoryStreamTransport(
            [
                b'{"sequence":2,"kind":"data","state":"Running","terminal":false}',
                b'{"sequence":2,"kind":"data","state":"Running","terminal":false}',
            ]
        )
        stream = StreamHandle.from_json(
            transport,
            b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
        )

        stream.next()
        with self.assertRaises(SDKError) as caught:
            stream.next()

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn("strictly ordered", str(caught.exception))
        self.assertEqual(stream.state, StreamState.FAILED)

    def test_transport_terminal_fails_stream_without_runtime_terminal(self) -> None:
        transport = MemoryStreamTransport(
            [
                b'{"sequence":1,"kind":"error","state":"Failed","terminal":false,'
                b'"transport_terminal":true,"error":{"code":"ROUTE_UNAVAILABLE"}}'
            ]
        )
        stream = StreamHandle.from_json(
            transport,
            b'{"stream_id":"stream-transport","state":"Open","max_buffered_events":4}',
        )

        event = stream.next()

        self.assertFalse(event.terminal)
        self.assertTrue(event.transport_terminal)
        self.assertEqual(stream.state, StreamState.FAILED)
        with self.assertRaises(SDKError):
            stream.terminal_event()

    def test_stream_cancel_is_non_terminal_request(self) -> None:
        transport = MemoryStreamTransport()
        stream = StreamHandle.from_json(
            transport,
            b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
        )

        outcome = stream.cancel("client stop")

        self.assertFalse(outcome.cancelled)
        self.assertFalse(outcome.terminal)
        self.assertEqual(outcome.state, StreamState.CANCEL_REQUESTED)
        self.assertEqual(stream.state, StreamState.CANCEL_REQUESTED)
        self.assertEqual(transport.cancel_reason, "client stop")

    def test_stream_cancel_rejects_terminal_outcome(self) -> None:
        transport = MemoryStreamTransport()
        transport.cancel_reply = (
            b'{"stream_id":"stream-1","cancelled":true,'
            b'"state":"Cancelled","terminal":true}'
        )
        stream = StreamHandle.from_json(
            transport,
            b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
        )

        with self.assertRaises(SDKError):
            stream.cancel("client stop")
        self.assertEqual(stream.state, StreamState.FAILED)

    def test_stream_unsupported_cancel_preserves_open_state(self) -> None:
        stream = StreamHandle.from_json(
            UnsupportedCancelStreamTransport(),
            b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
        )

        with self.assertRaises(SDKError) as caught:
            stream.cancel("client stop")

        self.assertTrue(is_code(caught.exception, ErrorCode.NOT_IMPLEMENTED))
        self.assertEqual(stream.state, StreamState.OPEN)
        stream.close()
        self.assertEqual(stream.state, StreamState.CLOSED)
        self.assertEqual(stream.runtime_state, StreamState.OPEN)

    def test_stream_interrupted_cancel_preserves_open_state(self) -> None:
        stream = StreamHandle.from_json(
            InterruptedCancelStreamTransport(),
            b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
        )

        with self.assertRaises(SDKError):
            stream.cancel("client stop")

        self.assertEqual(stream.state, StreamState.OPEN)
        self.assertEqual(stream.runtime_state, StreamState.OPEN)

    def test_cancel_while_receiving_preserves_canonical_terminal(self) -> None:
        transport = ConcurrentCancelStreamTransport()
        stream = StreamHandle.from_json(
            transport,
            b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
        )
        received: list[StreamEvent] = []
        errors: list[BaseException] = []

        receiver = threading.Thread(
            target=lambda: _capture_result(stream.next, received, errors),
            daemon=True,
        )
        receiver.start()
        self.assertTrue(transport.recv_started.wait(timeout=1.0))

        outcome = stream.cancel("client disconnected")
        receiver.join(timeout=1.0)

        self.assertFalse(receiver.is_alive())
        self.assertEqual(errors, [])
        self.assertEqual(outcome.state, StreamState.CANCEL_REQUESTED)
        self.assertEqual(len(received), 1)
        self.assertTrue(received[0].terminal)
        self.assertEqual(stream.state, StreamState.TERMINAL_FRAME_SEEN)
        self.assertEqual(
            stream.terminal_event().terminal_receipt,
            {"receipt_id": "cancelled-1"},
        )

    def test_second_concurrent_receiver_is_rejected(self) -> None:
        transport = ConcurrentCancelStreamTransport()
        stream = StreamHandle.from_json(
            transport,
            b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
        )
        received: list[StreamEvent] = []
        errors: list[BaseException] = []
        receiver = threading.Thread(
            target=lambda: _capture_result(stream.next, received, errors),
            daemon=True,
        )
        receiver.start()
        self.assertTrue(transport.recv_started.wait(timeout=1.0))

        with self.assertRaises(SDKError):
            stream.next()

        transport.terminal_ready.set()
        receiver.join(timeout=1.0)
        self.assertFalse(receiver.is_alive())
        self.assertEqual(errors, [])
        self.assertEqual(len(received), 1)

    def test_stream_enforces_buffer_bound(self) -> None:
        transport = MemoryStreamTransport(
            [
                b'{"sequence":1,"kind":"data","state":"Open","terminal":false}',
                b'{"sequence":2,"kind":"data","state":"Open","terminal":false}',
            ]
        )
        stream = StreamHandle.from_json(
            transport,
            b'{"stream_id":"stream-1","state":"Opening","max_buffered_events":1}',
        )

        stream.next()
        with self.assertRaises(SDKError) as caught:
            stream.next()

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(stream.state, StreamState.FAILED)


if __name__ == "__main__":
    unittest.main()
