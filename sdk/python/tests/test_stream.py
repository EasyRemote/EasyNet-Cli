import unittest

from easynet_sdk import (
    ErrorCode,
    SDKError,
    StreamEvent,
    StreamHandle,
    StreamState,
    StreamTerminalEvent,
    is_code,
)


class MemoryStreamTransport:
    def __init__(self, events: list[bytes] | None = None) -> None:
        self.events = list(events or [])
        self.closed = False
        self.cancel_reason = ""

    def recv(self, timeout: float | None = None) -> bytes:
        if not self.events:
            raise RuntimeError("no event")
        return self.events.pop(0)

    def cancel(self, reason: str) -> bytes:
        self.cancel_reason = reason
        return b'{"stream_id":"stream-1","cancelled":true,"state":"Cancelled","terminal":true}'

    def close(self) -> None:
        self.closed = True


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
                b'{"sequence":1,"kind":"chunk","state":"Open","terminal":false,'
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

    def test_stream_event_does_not_accept_legacy_content_type_alias(self) -> None:
        event = StreamEvent.from_json(
            b'{"sequence":1,"kind":"chunk","content_type":"application/json"}'
        )

        self.assertEqual(event.payload_content_type, "")

    def test_stream_terminal_event_projects_terminal_receipt(self) -> None:
        transport = MemoryStreamTransport(
            [
                b'{"sequence":1,"kind":"terminal","state":"Completed",'
                b'"terminal":true,"payload_json":{"ok":true},'
                b'"selected_node_id":"node-a","scheduling_reason":"local",'
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

        self.assertEqual(event.selected_node_id, "node-a")
        self.assertEqual(event.scheduling_reason, "local")
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

    def test_stream_event_ignores_legacy_receipt_only_field(self) -> None:
        event = StreamEvent.from_json(
            b'{"sequence":1,"kind":"terminal","state":"Completed",'
            b'"terminal":true,"receipt":{"receipt_id":"legacy-only"}}'
        )

        terminal = StreamTerminalEvent.from_event("stream-legacy", event)

        self.assertIsNone(event.terminal_receipt)
        self.assertFalse(hasattr(event, "receipt"))
        self.assertIsNone(terminal.terminal_receipt)

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

    def test_stream_cancels_non_terminal_stream(self) -> None:
        transport = MemoryStreamTransport()
        stream = StreamHandle.from_json(
            transport,
            b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
        )

        outcome = stream.cancel("client stop")

        self.assertTrue(outcome.cancelled)
        self.assertTrue(outcome.terminal)
        self.assertEqual(stream.state, StreamState.CANCELLED)
        self.assertEqual(transport.cancel_reason, "client stop")

    def test_stream_enforces_buffer_bound(self) -> None:
        transport = MemoryStreamTransport(
            [
                b'{"sequence":1,"kind":"chunk","state":"Open","terminal":false}',
                b'{"sequence":2,"kind":"chunk","state":"Open","terminal":false}',
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
