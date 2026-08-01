import json
import threading
import unittest
from collections.abc import Callable

from easynet_sdk import (
    BidiFrame,
    BidiOutcome,
    BidiSession,
    BidiState,
    BidiTerminalFrame,
    ErrorCode,
    RetryHint,
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


class UnsupportedCancelBidiTransport(MemoryBidiTransport):
    def cancel(self, reason: str) -> bytes:
        del reason
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="bidi cancellation unsupported",
        )


class InterruptedSendBidiTransport(MemoryBidiTransport):
    def send(self, frame_json: bytes) -> bytes:
        del frame_json
        raise TimeoutError("send deadline elapsed")


class InterruptedCloseSendBidiTransport(MemoryBidiTransport):
    def close_send(self) -> bytes:
        raise SDKError(
            code=ErrorCode.TIMEOUT,
            stage="test",
            retry=RetryHint.SAFE,
            message="close-send deadline elapsed",
        )


class InterruptedCancelBidiTransport(MemoryBidiTransport):
    def cancel(self, reason: str) -> bytes:
        del reason
        raise SDKError(
            code=ErrorCode.CANCELLED,
            stage="test",
            retry=RetryHint.NEVER,
            message="cancel request interrupted",
        )


class ConcurrentCancelBidiTransport(MemoryBidiTransport):
    def __init__(self) -> None:
        super().__init__()
        self.recv_started = threading.Event()
        self.terminal_ready = threading.Event()
        self.recv_frame = (
            b'{"sequence":1,"kind":"terminal","stream_id":1,"terminal":true,'
            b'"terminal_receipt":{"receipt_id":"cancelled-1"}}'
        )

    def recv(self, timeout: float | None = None) -> bytes:
        self.recv_started.set()
        if not self.terminal_ready.wait(timeout=timeout or 1.0):
            raise TimeoutError("terminal frame was not released")
        return self.recv_frame

    def cancel(self, reason: str) -> bytes:
        self.cancel_reason = reason
        self.terminal_ready.set()
        return self.cancel_reply


class FailedSendConcurrentBidiTransport(ConcurrentCancelBidiTransport):
    def send(self, frame_json: bytes) -> bytes:
        del frame_json
        raise SDKError(
            code=ErrorCode.TRANSPORT,
            stage="test",
            retry=RetryHint.SAFE,
            message="carrier send failed",
        )


def _capture_result(
    operation: Callable[[], BidiFrame],
    results: list[BidiFrame],
    errors: list[BaseException],
) -> None:
    try:
        results.append(operation())
    except BaseException as exc:
        errors.append(exc)


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

    def test_local_operation_interruptions_preserve_open_state(self) -> None:
        operations: tuple[
            tuple[MemoryBidiTransport, Callable[[BidiSession], object]], ...
        ] = (
            (
                InterruptedSendBidiTransport(),
                lambda session: session.send(
                    BidiFrame(sequence=1, kind="data", stream_id=1)
                ),
            ),
            (
                InterruptedCloseSendBidiTransport(),
                lambda session: session.close_send(),
            ),
            (
                InterruptedCancelBidiTransport(),
                lambda session: session.cancel("client stop"),
            ),
        )
        for transport, operation in operations:
            with self.subTest(transport=type(transport).__name__):
                session = new_session(transport)
                with self.assertRaises((SDKError, TimeoutError)):
                    operation(session)
                self.assertEqual(session.state, BidiState.OPEN)
                self.assertEqual(session.runtime_state, BidiState.OPEN)

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

    def test_bidi_projections_reject_product_state_code(self) -> None:
        with self.assertRaises(SDKError) as open_caught:
            BidiSession.from_json(
                MemoryBidiTransport(),
                b'{"session_id":"bidi-1","state":"Open",'
                b'"max_buffered_frames":4,"state_code":"B200"}',
            )
        self.assertIn(
            "bidi open contains noncanonical field state_code",
            str(open_caught.exception),
        )
        with self.assertRaises(SDKError) as frame_caught:
            BidiFrame.from_json(
                b'{"sequence":1,"kind":"data","stream_id":1,'
                b'"terminal":false,"state_code":"B200"}'
            )
        self.assertIn(
            "bidi frame contains noncanonical field state_code",
            str(frame_caught.exception),
        )
        with self.assertRaises(SDKError) as mac_caught:
            BidiFrame.from_json(
                b'{"sequence":1,"kind":"data","stream_id":1,'
                b'"terminal":false,'
                b'"mac_base64":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="}'
            )
        self.assertIn(
            "bidi frame contains noncanonical field mac_base64",
            str(mac_caught.exception),
        )
        with self.assertRaises(SDKError) as outcome_caught:
            BidiOutcome.from_json(
                b'{"session_id":"bidi-1","state":"CancelRequested",'
                b'"terminal":false,"reason":"stop","state_code":"B200"}'
            )
        self.assertIn(
            "bidi outcome contains noncanonical field state_code",
            str(outcome_caught.exception),
        )

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

    def test_half_close_waits_for_canonical_terminal_frame(self) -> None:
        transport = MemoryBidiTransport(
            [
                b'{"sequence":1,"kind":"remote_close_send","stream_id":1}',
                b'{"sequence":2,"kind":"terminal","stream_id":1,"terminal":true,'
                b'"terminal_receipt":{"receipt_id":"completed-1"}}',
            ]
        )
        session = new_session(transport)

        frame = session.receive()
        outcome = session.close_send()
        with self.assertRaises(SDKError):
            session.terminal_frame()
        terminal = session.receive()
        session.close()

        self.assertEqual(frame.kind, "remote_close_send")
        self.assertEqual(outcome.state, BidiState.HALF_CLOSED_LOCAL)
        self.assertFalse(outcome.terminal)
        self.assertTrue(terminal.terminal)
        self.assertEqual(
            terminal.terminal_receipt,
            {"receipt_id": "completed-1"},
        )
        self.assertTrue(transport.closed)
        self.assertEqual(session.state, BidiState.CLOSED)
        self.assertEqual(session.runtime_state, BidiState.TERMINAL)

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
        self.assertEqual(session.runtime_state, BidiState.CANCEL_REQUESTED)

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

    def test_unsupported_cancel_preserves_open_state(self) -> None:
        session = new_session(UnsupportedCancelBidiTransport())

        with self.assertRaises(SDKError) as caught:
            session.cancel("client stop")

        self.assertTrue(is_code(caught.exception, ErrorCode.NOT_IMPLEMENTED))
        self.assertEqual(session.state, BidiState.OPEN)
        session.close()
        self.assertTrue(session.transport.closed)
        self.assertEqual(session.state, BidiState.CLOSED)
        self.assertEqual(session.runtime_state, BidiState.OPEN)

    def test_cancel_while_receiving_preserves_canonical_terminal(self) -> None:
        transport = ConcurrentCancelBidiTransport()
        session = new_session(transport)
        received: list[BidiFrame] = []
        errors: list[BaseException] = []
        receiver = threading.Thread(
            target=lambda: _capture_result(session.receive, received, errors),
            daemon=True,
        )
        receiver.start()
        self.assertTrue(transport.recv_started.wait(timeout=1.0))

        outcome = session.cancel("client disconnected")
        receiver.join(timeout=1.0)

        self.assertFalse(receiver.is_alive())
        self.assertEqual(errors, [])
        self.assertEqual(outcome.state, BidiState.CANCEL_REQUESTED)
        self.assertEqual(len(received), 1)
        self.assertTrue(received[0].terminal)
        self.assertEqual(session.state, BidiState.TERMINAL)
        self.assertEqual(
            session.terminal_frame().terminal_receipt,
            {"receipt_id": "cancelled-1"},
        )

    def test_receive_returns_terminal_frame_when_session_already_terminal_in_flight(
        self,
    ) -> None:
        transport = ConcurrentCancelBidiTransport()
        session = new_session(transport)
        received: list[BidiFrame] = []
        errors: list[BaseException] = []
        receiver = threading.Thread(
            target=lambda: _capture_result(session.receive, received, errors),
            daemon=True,
        )
        receiver.start()
        self.assertTrue(transport.recv_started.wait(timeout=1.0))

        with session._lock:
            session._set_runtime_state_locked(BidiState.TERMINAL)
        transport.terminal_ready.set()
        receiver.join(timeout=1.0)

        self.assertFalse(receiver.is_alive())
        self.assertEqual(errors, [])
        self.assertEqual(len(received), 1)
        self.assertTrue(received[0].terminal)
        self.assertEqual(session.state, BidiState.TERMINAL)

    def test_receive_rejects_non_terminal_frame_when_session_already_terminal_in_flight(
        self,
    ) -> None:
        transport = ConcurrentCancelBidiTransport()
        transport.recv_frame = (
            b'{"sequence":1,"kind":"data","stream_id":1,'
            b'"payload_json":{"late":true}}'
        )
        session = new_session(transport)
        errors: list[BaseException] = []
        receiver = threading.Thread(
            target=lambda: _capture_result(session.receive, [], errors),
            daemon=True,
        )
        receiver.start()
        self.assertTrue(transport.recv_started.wait(timeout=1.0))

        with session._lock:
            session._set_runtime_state_locked(BidiState.TERMINAL)
        transport.terminal_ready.set()
        receiver.join(timeout=1.0)

        self.assertFalse(receiver.is_alive())
        self.assertEqual(len(errors), 1)
        self.assertIn("became terminal", str(errors[0]))

    def test_in_flight_receive_drains_after_carrier_send_failure(self) -> None:
        transport = FailedSendConcurrentBidiTransport()
        transport.recv_frame = (
            b'{"sequence":1,"kind":"data","stream_id":1,'
            b'"payload_json":{"ready":true}}'
        )
        session = new_session(transport)
        received: list[BidiFrame] = []
        errors: list[BaseException] = []
        receiver = threading.Thread(
            target=lambda: _capture_result(session.receive, received, errors),
            daemon=True,
        )
        receiver.start()
        self.assertTrue(transport.recv_started.wait(timeout=1.0))

        with self.assertRaises(SDKError) as caught:
            session.send(BidiFrame(sequence=1, kind="data", stream_id=1))
        self.assertTrue(is_code(caught.exception, ErrorCode.TRANSPORT))
        self.assertEqual(session.state, BidiState.FAILED)
        self.assertEqual(session.runtime_state, BidiState.OPEN)

        transport.terminal_ready.set()
        receiver.join(timeout=1.0)
        self.assertFalse(receiver.is_alive())
        self.assertEqual(errors, [])
        self.assertEqual([frame.sequence for frame in received], [1])
        with self.assertRaises(SDKError):
            session.receive()

    def test_second_concurrent_receiver_is_rejected(self) -> None:
        transport = ConcurrentCancelBidiTransport()
        session = new_session(transport)
        received: list[BidiFrame] = []
        errors: list[BaseException] = []
        receiver = threading.Thread(
            target=lambda: _capture_result(session.receive, received, errors),
            daemon=True,
        )
        receiver.start()
        self.assertTrue(transport.recv_started.wait(timeout=1.0))

        with self.assertRaises(SDKError):
            session.receive()

        transport.terminal_ready.set()
        receiver.join(timeout=1.0)
        self.assertFalse(receiver.is_alive())
        self.assertEqual(errors, [])
        self.assertEqual(len(received), 1)

    def test_receive_history_is_bounded_rolling_window(self) -> None:
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
        session.receive()

        self.assertEqual([frame.sequence for frame in session.received_frames], [2])
        self.assertEqual(session.state, BidiState.OPEN)
        self.assertEqual(session.runtime_state, BidiState.OPEN)

    def test_send_history_is_bounded_rolling_window(self) -> None:
        session = BidiSession.from_json(
            MemoryBidiTransport(),
            b'{"session_id":"bidi-1","state":"Open","max_buffered_frames":1}',
        )

        session.send(BidiFrame(sequence=1, kind="data", stream_id=1))
        session.send(BidiFrame(sequence=2, kind="data", stream_id=1))

        self.assertEqual([frame.sequence for frame in session.sent_frames], [2])
        self.assertEqual(session.state, BidiState.OPEN)


if __name__ == "__main__":
    unittest.main()
