import json
import unittest

from easynet_sdk import (
    BidiFrame,
    BidiSession,
    BidiState,
    ErrorCode,
    SDKError,
    is_code,
)


class MemoryBidiTransport:
    def __init__(self, recv_frames: list[bytes] | None = None) -> None:
        self.recv_frames = list(recv_frames or [])
        self.sent_frames: list[dict[str, object]] = []
        self.closed = False
        self.cancel_reason = ""

    def send(self, frame_json: bytes) -> bytes:
        self.sent_frames.append(json.loads(frame_json.decode("utf-8")))
        return frame_json

    def recv(self, timeout: float | None = None) -> bytes:
        if not self.recv_frames:
            raise RuntimeError("no frame")
        return self.recv_frames.pop(0)

    def close_send(self) -> bytes:
        return b'{"session_id":"bidi-1","state":"HalfClosedLocal","terminal":false}'

    def close(self) -> None:
        self.closed = True

    def cancel(self, reason: str) -> bytes:
        self.cancel_reason = reason
        return b'{"session_id":"bidi-1","state":"Cancelled","terminal":true,"reason":"client stop"}'


def new_session(transport: MemoryBidiTransport) -> BidiSession:
    return BidiSession.from_json(
        transport,
        b'{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}',
    )


class BidiTests(unittest.TestCase):
    def test_bidi_receive_timeout_does_not_change_state(self) -> None:
        class TimeoutBidiTransport(MemoryBidiTransport):
            def recv(self, timeout: float | None = None) -> bytes:
                self.timeout = timeout
                raise TimeoutError("no frame")

        transport = TimeoutBidiTransport()
        session = new_session(transport)

        with self.assertRaises(TimeoutError):
            session.receive(timeout=0.01)

        self.assertEqual(transport.timeout, 0.01)
        self.assertEqual(session.state, BidiState.OPEN)

    def test_bidi_sends_and_receives_ordered_frames(self) -> None:
        transport = MemoryBidiTransport(
            [b'{"sequence":1,"kind":"data","stream_id":1,"payload_json":{"ready":true}}']
        )
        session = new_session(transport)

        ack = session.send(BidiFrame(sequence=1, kind="data", stream_id=1))
        received = session.receive()

        self.assertEqual(ack.sequence, 1)
        self.assertEqual(received.sequence, 1)
        self.assertEqual(session.state, BidiState.OPEN)

    def test_close_send_differs_from_cancel(self) -> None:
        transport = MemoryBidiTransport()
        session = new_session(transport)

        outcome = session.close_send()

        self.assertEqual(outcome.state, BidiState.HALF_CLOSED_LOCAL)
        self.assertFalse(outcome.terminal)
        self.assertEqual(session.state, BidiState.HALF_CLOSED_LOCAL)
        with self.assertRaises(SDKError) as caught:
            session.send(BidiFrame(sequence=1, kind="data", stream_id=1))
        self.assertEqual(caught.exception.code, ErrorCode.CANCELLED)
        self.assertEqual(session.state, BidiState.HALF_CLOSED_LOCAL)

    def test_remote_close_then_local_close_send_reaches_terminal(self) -> None:
        transport = MemoryBidiTransport(
            [b'{"sequence":1,"kind":"remote_close_send","stream_id":1}']
        )
        session = new_session(transport)

        frame = session.receive()
        outcome = session.close_send()
        session.close()

        self.assertEqual(frame.kind, "remote_close_send")
        self.assertEqual(outcome.state, BidiState.TERMINAL)
        self.assertTrue(outcome.terminal)
        self.assertTrue(transport.closed)
        self.assertEqual(session.state, BidiState.CLOSED)

    def test_cancel_is_terminal(self) -> None:
        transport = MemoryBidiTransport()
        session = new_session(transport)

        outcome = session.cancel("client stop")

        self.assertEqual(outcome.state, BidiState.CANCELLED)
        self.assertTrue(outcome.terminal)
        self.assertEqual(transport.cancel_reason, "client stop")
        with self.assertRaises(SDKError):
            session.close_send()

    def test_receive_buffer_bound(self) -> None:
        transport = MemoryBidiTransport(
            [
                b'{"sequence":1,"kind":"data","stream_id":1}',
                b'{"sequence":2,"kind":"data","stream_id":1}',
            ]
        )
        session = BidiSession.from_json(
            transport,
            b'{"session_id":"bidi-1","state":"Open","max_buffered_frames":1}',
        )

        session.receive()
        with self.assertRaises(SDKError) as caught:
            session.receive()

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(session.state, BidiState.FAILED)


if __name__ == "__main__":
    unittest.main()
