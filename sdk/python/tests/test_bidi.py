import json
import unittest

from easynet_sdk import (
    BidiFrame,
    BidiSession,
    BidiState,
    BidiTerminalFrame,
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
        self.cancel_reply = (
            b'{"session_id":"bidi-1","state":"CancelRequested",'
            b'"terminal":false,"reason":"client stop"}'
        )

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
        return self.cancel_reply


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
            [
                b'{"sequence":1,"kind":"data","stream_id":1,"payload_json":{"ready":true}}'
            ]
        )
        session = new_session(transport)

        ack = session.send(BidiFrame(sequence=1, kind="data", stream_id=1))
        received = session.receive()

        self.assertEqual(ack.sequence, 1)
        self.assertEqual(received.sequence, 1)
        self.assertEqual(session.state, BidiState.OPEN)

    def test_bidi_frame_preserves_finalization_checkpoints(self) -> None:
        frame = BidiFrame.from_json(
            b'{"sequence":9,"kind":"terminal","stream_id":1,"terminal":true,'
            b'"admission_receipt":{"invocation_id":"inv-1","index":3,'
            b'"authority_proof":{"proof_type":"signed"}},'
            b'"terminal_receipt":{"invocation_id":"inv-1","index":8,'
            b'"output_hash":"abcd"}}'
        )

        self.assertEqual(frame.admission_receipt["index"], 3)
        self.assertEqual(frame.terminal_receipt["index"], 8)
        encoded = json.loads(frame.to_json())
        self.assertEqual(
            encoded["admission_receipt"]["authority_proof"]["proof_type"],
            "signed",
        )
        self.assertEqual(encoded["terminal_receipt"]["output_hash"], "abcd")

    def test_bidi_frame_rejects_legacy_event_alias(self) -> None:
        with self.assertRaises(SDKError) as caught:
            BidiFrame.from_json(b'{"sequence":1,"event":"data","stream_id":1}')

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_transport_terminal_fails_session_without_runtime_terminal(self) -> None:
        transport = MemoryBidiTransport(
            [
                b'{"sequence":1,"kind":"error","stream_id":1,"terminal":false,'
                b'"transport_terminal":true,"error":{"code":"ROUTE_UNAVAILABLE"}}'
            ]
        )
        session = new_session(transport)

        frame = session.receive()

        self.assertFalse(frame.terminal)
        self.assertTrue(frame.transport_terminal)
        self.assertEqual(session.state, BidiState.FAILED)
        with self.assertRaises(SDKError):
            session.terminal_frame()

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

    def test_terminal_frame_projects_schema_shape(self) -> None:
        transport = MemoryBidiTransport(
            [
                b'{"sequence":1,"kind":"terminal","stream_id":1,"terminal":true,'
                b'"payload_json":{"ok":true},"terminal_receipt":{"receipt_ura":'
                b'"easynet:///r/example/resource/agent.alice.sdk/invocation/r1/receipt"}}'
            ]
        )
        session = new_session(transport)

        session.receive()
        terminal = session.terminal_frame()

        self.assertIsInstance(terminal, BidiTerminalFrame)
        self.assertEqual(terminal.session_id, "bidi-1")
        self.assertEqual(terminal.frame_type, "terminal")
        self.assertEqual(terminal.seq, 1)
        self.assertEqual(
            terminal.terminal_receipt,
            {
                "receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/r1/receipt"
            },
        )
        self.assertIn(b'"frame_type":"terminal"', terminal.to_json())
        self.assertIn(b'"terminal_receipt":', terminal.to_json())
        self.assertNotIn(b'"receipt":', terminal.to_json())

    def test_bidi_frame_rejects_legacy_receipt_only_field(self) -> None:
        with self.assertRaises(SDKError) as caught:
            BidiFrame.from_json(
                b'{"sequence":2,"kind":"terminal","stream_id":1,"terminal":true,'
                b'"receipt":{"receipt_id":"legacy-only"}}'
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_cancel_is_non_terminal_request(self) -> None:
        transport = MemoryBidiTransport()
        session = new_session(transport)

        outcome = session.cancel("client stop")

        self.assertEqual(outcome.state, BidiState.CANCEL_REQUESTED)
        self.assertFalse(outcome.terminal)
        self.assertEqual(transport.cancel_reason, "client stop")
        with self.assertRaises(SDKError):
            session.close_send()
        session.close()
        self.assertTrue(transport.closed)
        self.assertEqual(session.state, BidiState.CLOSED)

    def test_cancel_rejects_terminal_outcome(self) -> None:
        transport = MemoryBidiTransport()
        transport.cancel_reply = (
            b'{"session_id":"bidi-1","state":"Cancelled",'
            b'"terminal":true,"reason":"client stop"}'
        )
        session = new_session(transport)

        with self.assertRaises(SDKError):
            session.cancel("client stop")
        self.assertEqual(session.state, BidiState.FAILED)

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
