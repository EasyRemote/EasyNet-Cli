import unittest

from easynet_sdk import (
    ErrorCode,
    SDKError,
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
                b'{"sequence":1,"event":"chunk","state":"Open","terminal":false,'
                b'"payload_content_type":"application/json","payload_json":{"step":1}}',
                b'{"sequence":2,"event":"terminal","state":"Completed","terminal":true,'
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

    def test_stream_terminal_event_projects_receipt_payload(self) -> None:
        transport = MemoryStreamTransport(
            [
                b'{"sequence":1,"event":"terminal","state":"Completed",'
                b'"terminal":true,"payload_json":{"receipt":{"receipt_ura":'
                b'"easynet:///r/example/resource/agent.alice.sdk/invocation/r1/receipt"}}}'
            ]
        )
        stream = StreamHandle.from_json(
            transport,
            b'{"stream_id":"stream-1","state":"Open","max_buffered_events":4}',
        )

        stream.next()
        terminal = stream.terminal_event()

        self.assertEqual(
            terminal.receipt, {"receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/r1/receipt"}
        )
        self.assertIn(b'"event_type":"terminal"', terminal.to_json())

    def test_stream_rejects_next_after_terminal(self) -> None:
        transport = MemoryStreamTransport(
            [
                b'{"sequence":1,"event":"terminal","state":"Completed","terminal":true}',
                b'{"sequence":2,"event":"terminal","state":"Completed","terminal":true}',
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
                b'{"sequence":1,"event":"chunk","state":"Open","terminal":false}',
                b'{"sequence":2,"event":"chunk","state":"Open","terminal":false}',
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
