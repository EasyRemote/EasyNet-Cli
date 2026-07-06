import json
import tempfile
import threading
import time
import unittest
from collections.abc import Mapping
from pathlib import Path
from unittest.mock import patch

from easynet_sdk import (
    BidiState,
    DaemonInvocationTransport,
    BidiSessionAdapter,
    StreamValueAdapter,
    InvocationResultAdapter,
    UnaryDispatchPool,
    ErrorCode,
    InvocationSignature,
    RetryHint,
    RuntimeClient,
    SDKError,
    Signer,
    StreamState,
    is_code,
)
from easynet_sdk._cabi import CLILibrary

from test_cabi import FakeRawCABI
from test_runtime import MemoryRuntimeTransport, complete_draft
from test_signing import signer_handle


def _load_patch(raw: FakeRawCABI):
    return patch("easynet_sdk._cabi.CLILibrary.load", return_value=CLILibrary(raw))


class DaemonInvocationTransportTests(unittest.TestCase):
    def test_invoke_accepts_complete_invocation_mapping(self) -> None:
        runtime = MemoryRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(
            RuntimeClient(runtime)
        )

        result = transport.invoke(complete_draft().to_json_dict())

        self.assertTrue(result["ok"])
        self.assertEqual(result["output_json"], {"ready": True})
        self.assertEqual(result["terminal_state"], "Completed")
        assert runtime.seen_draft is not None
        self.assertEqual(
            runtime.seen_draft["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )

    def test_invocation_result_adapter_projects_runtime_result_shape(self) -> None:
        class ResultShapeRuntimeTransport(MemoryRuntimeTransport):
            def invoke(self, draft_json: bytes) -> bytes:
                result = json.loads(super().invoke(draft_json).decode("utf-8"))
                result.update(
                    {
                        "terminal_state": "Completed",
                        "output_content_type": "application/json",
                        "output_base64": "eyJyZWFkeSI6dHJ1ZX0=",
                        "output_json": {"ready": True},
                        "selected_node_id": "dev-a",
                        "scheduling_reason": "direct",
                        "elapsed_ms": 12,
                        "receipt": {"invocation_id": "inv-1"},
                    }
                )
                return json.dumps(
                    result,
                    separators=(",", ":"),
                    sort_keys=True,
                ).encode("utf-8")

        runtime = ResultShapeRuntimeTransport()
        adapter = InvocationResultAdapter.from_runtime_client(RuntimeClient(runtime))

        result = adapter.invoke(complete_draft().to_json_dict())

        self.assertTrue(result["ok"])
        self.assertEqual(result["state"], 5)
        self.assertEqual(result["terminal_state"], "Completed")
        self.assertEqual(result["result_json"], {"ready": True})
        self.assertEqual(result["result_base64"], "eyJyZWFkeSI6dHJ1ZX0=")
        self.assertEqual(result["result_content_type"], "application/json")
        self.assertEqual(result["selected_node_id"], "dev-a")
        self.assertEqual(result["scheduling_reason"], "direct")
        self.assertEqual(result["elapsed_ms"], 12)
        self.assertEqual(result["admission_receipt"], {"invocation_id": "inv-1"})
        self.assertEqual(result["sdk_runtime_result"]["terminal_state"], "Completed")

    def test_invocation_result_adapter_requires_sdk_signer_for_signed_invocation(
        self,
    ) -> None:
        runtime = MemoryRuntimeTransport()
        adapter = InvocationResultAdapter.from_runtime_client(RuntimeClient(runtime))

        with self.assertRaises(SDKError) as caught:
            adapter.invoke_signed(complete_draft().to_json_dict(), signer=None)

        self.assertTrue(is_code(caught.exception, ErrorCode.NOT_IMPLEMENTED))
        self.assertEqual(caught.exception.stage, "runtime_signing")
        self.assertEqual(caught.exception.details["reason"], "signing_path_pending")
        self.assertIsNone(runtime.seen_draft)

    def test_invocation_result_adapter_submits_signed_invocation_through_runtime(
        self,
    ) -> None:
        runtime = MemoryRuntimeTransport()
        adapter = InvocationResultAdapter.from_runtime_client(RuntimeClient(runtime))
        signer = Signer.from_signature(
            signer_handle(),
            InvocationSignature(
                algorithm="ed25519",
                signature_base64="c2lnbmF0dXJl",
            ),
        )

        result = adapter.invoke_signed(complete_draft().to_json_dict(), signer=signer)

        self.assertTrue(result["ok"])
        self.assertEqual(result["terminal_state"], "Completed")
        self.assertEqual(result["state"], 5)
        self.assertEqual(runtime.seen_options, {"local_daemon_signing": True})
        self.assertEqual(runtime.seen_await_id, 7)
        self.assertEqual(runtime.seen_free_id, 7)
        assert runtime.seen_signed is not None
        self.assertEqual(runtime.seen_signed["signer_id"], "signer-alice-key-1")

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
                return json.dumps(result, separators=(",", ":"), sort_keys=True).encode(
                    "utf-8"
                )

        runtime = ReceiptRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(
            RuntimeClient(runtime)
        )

        result = transport.invoke(complete_draft().to_json_dict())

        self.assertEqual(result["receipt"]["invocation_id"], "inv-1")
        self.assertEqual(result["receipt_summary"]["invocation_id"], "inv-1")
        self.assertTrue(result["receipt_summary"]["has_causal_anchor"])

    def test_invocation_result_adapter_raises_on_non_ok_runtime_result(self) -> None:
        class FailedRuntimeTransport(MemoryRuntimeTransport):
            def invoke(self, draft_json: bytes) -> bytes:
                draft = json.loads(draft_json.decode("utf-8"))
                return json.dumps(
                    {
                        "ok": False,
                        "tuple": draft,
                        "terminal_state": "Failed",
                        "error": {
                            "code": "ABILITY_FAILED",
                            "stage": "runtime",
                            "message": "ability failed",
                            "retryable": False,
                        },
                    },
                    separators=(",", ":"),
                    sort_keys=True,
                ).encode("utf-8")

        adapter = InvocationResultAdapter.from_runtime_client(
            RuntimeClient(FailedRuntimeTransport())
        )

        with self.assertRaises(SDKError) as caught:
            adapter.invoke(complete_draft())

        self.assertTrue(is_code(caught.exception, ErrorCode.ABILITY_FAILED))
        self.assertEqual(caught.exception.message, "ability failed")

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
        transport = DaemonInvocationTransport.from_runtime_client(
            RuntimeClient(runtime)
        )

        stream = transport.stream(complete_draft())
        event = stream.recv()
        stream.close()

        self.assertTrue(event["terminal"])
        self.assertEqual(event["kind"], "terminal")

    def test_invocation_result_adapter_delegates_stream_and_bidi(self) -> None:
        runtime = MemoryRuntimeTransport()
        adapter = InvocationResultAdapter.from_runtime_client(RuntimeClient(runtime))

        stream = adapter.stream(complete_draft().to_json_dict())
        event = stream.recv()
        channel = adapter.bidi(
            complete_draft().to_json_dict(),
            [{"stream_id": 1, "content_type": "application/json"}],
        )
        ack = channel.send({"sequence": 1, "kind": "data", "stream_id": 1})
        channel.cancel("done")
        adapter.close()
        adapter.close()

        self.assertTrue(event["terminal"])
        self.assertIn("content_type", event)
        self.assertEqual(ack["sequence"], 1)
        self.assertEqual(runtime.close_calls, 1)

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
        transport = DaemonInvocationTransport.from_runtime_client(
            RuntimeClient(runtime)
        )

        stream = transport.stream(complete_draft())
        with self.assertRaises(TimeoutError):
            stream.recv(timeout=0.01)

        self.assertEqual(runtime.stream_transport.timeout, 0.01)
        self.assertEqual(stream.handle.state, StreamState.OPEN)

    def test_bidi_keeps_half_close_cancel_and_close_distinct(self) -> None:
        runtime = MemoryRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(
            RuntimeClient(runtime)
        )
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
        transport = DaemonInvocationTransport.from_runtime_client(
            RuntimeClient(runtime)
        )

        channel = transport.bidi(
            complete_draft().to_json_dict(),
            [{"stream_id": 1, "content_type": "application/json"}],
        )
        with self.assertRaises(TimeoutError):
            channel.recv(timeout=0.01)

        self.assertEqual(runtime.bidi_transport.timeout, 0.01)
        self.assertEqual(channel.session.state, BidiState.OPEN)

    def test_bidi_close_cancels_open_session_before_release(self) -> None:
        channel = _MemoryBidiChannel(close_requires_terminal=True)
        session = BidiSessionAdapter(channel)

        session.close()
        session.close()

        self.assertEqual(channel.cancel_reasons, ["client close"])
        self.assertEqual(channel.close_calls, 2)
        self.assertTrue(channel.closed)

    def test_bidi_close_preserves_unrelated_invalid_argument(self) -> None:
        channel = _MemoryBidiChannel(close_error="invalid frame state")
        session = BidiSessionAdapter(channel)

        with self.assertRaises(SDKError) as caught:
            session.close()

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(channel.cancel_reasons, [])
        self.assertEqual(channel.close_calls, 1)

    def test_bidi_cancel_does_not_close_transport(self) -> None:
        channel = _MemoryBidiChannel()
        session = BidiSessionAdapter(channel)

        session.cancel("user stop")

        self.assertEqual(channel.cancel_reasons, ["user stop"])
        self.assertEqual(channel.close_calls, 0)
        self.assertFalse(channel.closed)

    def test_bidi_recv_timeout_is_typed_client_wait(self) -> None:
        channel = _MemoryBidiChannel(timeout=True)
        session = BidiSessionAdapter(channel)

        with self.assertRaises(SDKError) as caught:
            session.recv(timeout=0.01)

        self.assertTrue(is_code(caught.exception, ErrorCode.TIMEOUT))
        self.assertEqual(caught.exception.stage, "bidi")
        self.assertEqual(caught.exception.details["reason"], "client_wait_timeout")

    def test_bidi_recv_remote_error_is_typed(self) -> None:
        channel = _MemoryBidiChannel(
            frames=[
                {
                    "sequence": 1,
                    "kind": "data",
                    "stream_id": 1,
                    "error": {
                        "kind": "UNAVAILABLE",
                        "reason": "host_gone",
                        "message": "host went away",
                    },
                }
            ]
        )
        session = BidiSessionAdapter(channel)

        with self.assertRaises(SDKError) as caught:
            session.recv()

        self.assertTrue(is_code(caught.exception, ErrorCode.DAEMON_OFFLINE))
        self.assertEqual(caught.exception.stage, "bidi")
        self.assertEqual(caught.exception.details["reason"], "host_gone")

    def test_bidi_rejects_send_after_close(self) -> None:
        channel = _MemoryBidiChannel()
        session = BidiSessionAdapter(channel)

        session.close()
        with self.assertRaises(SDKError) as caught:
            session.send({"sequence": 1, "kind": "data", "stream_id": 1})

        self.assertTrue(is_code(caught.exception, ErrorCode.CANCELLED))
        self.assertEqual(caught.exception.stage, "bidi")

    def test_rejects_incomplete_invocation_mapping_before_dispatch(self) -> None:
        runtime = MemoryRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(
            RuntimeClient(runtime)
        )

        with self.assertRaises(SDKError) as caught:
            transport.invoke({"caller_ura": "easynet:///r/example/agent/alice.sdk"})

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(runtime.seen_draft)

    def test_unary_pool_retires_timed_out_owned_transport(self) -> None:
        first = _SlowUnaryTransport(delay=0.05)
        second = _SlowUnaryTransport()
        transports = [first, second]
        pool = UnaryDispatchPool(lambda: transports.pop(0))

        with self.assertRaises(SDKError) as caught:
            pool.invoke(complete_draft().to_json_dict(), timeout=0.001)

        self.assertTrue(is_code(caught.exception, ErrorCode.TIMEOUT))
        self.assertEqual(caught.exception.details["reason"], "client_wait_timeout")
        self.assertIsNone(pool.current_transport)

        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertTrue(result["ok"])
        self.assertEqual(transports, [])
        self.assertIs(pool.current_transport, second)
        _wait_until(lambda: first.closed)
        self.assertTrue(first.closed)
        self.assertFalse(second.closed)

    def test_unary_pool_signed_dispatch_reuses_wait_state(self) -> None:
        transport = _SlowUnaryTransport()
        pool = UnaryDispatchPool.from_transport(transport)
        signer = Signer.from_signature(
            signer_handle(),
            InvocationSignature(
                algorithm="ed25519",
                signature_base64="c2lnbmF0dXJl",
            ),
        )

        result = pool.invoke_signed(
            complete_draft().to_json_dict(),
            signer=signer,
            timeout=1.0,
        )

        self.assertEqual(result, {"ok": True})
        self.assertEqual(len(transport.invocations), 1)
        self.assertEqual(transport.signed_signers, [signer])

    def test_unary_pool_queue_timeout_does_not_retire_active_transport(
        self,
    ) -> None:
        first = _SlowUnaryTransport(delay=0.05)
        second = _SlowUnaryTransport()
        transports = [first, second]
        pool = UnaryDispatchPool(lambda: transports.pop(0))
        result: list[dict[str, object]] = []
        thread = threading.Thread(
            target=lambda: result.append(
                pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
            ),
            daemon=True,
        )

        thread.start()
        self.assertTrue(first.started.wait(timeout=1.0))
        with self.assertRaises(SDKError) as caught:
            pool.invoke(complete_draft().to_json_dict(), timeout=0.001)
        thread.join(timeout=1.0)

        self.assertTrue(is_code(caught.exception, ErrorCode.TIMEOUT))
        self.assertEqual(result, [{"ok": True}])
        self.assertEqual(len(first.invocations), 1)
        self.assertFalse(first.closed)
        self.assertIs(pool.current_transport, first)
        self.assertEqual(transports, [second])

    def test_unary_pool_close_during_active_invoke_is_bounded(self) -> None:
        transport = _SlowUnaryTransport(delay=0.05)
        pool = UnaryDispatchPool(lambda: transport)
        result: list[dict[str, object]] = []
        thread = threading.Thread(
            target=lambda: result.append(
                pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
            ),
            daemon=True,
        )

        thread.start()
        self.assertTrue(transport.started.wait(timeout=1.0))
        started = time.perf_counter()
        pool.close()
        elapsed = time.perf_counter() - started

        self.assertLess(elapsed, 0.02)
        self.assertIsNone(pool.current_transport)
        thread.join(timeout=1.0)
        self.assertEqual(result, [{"ok": True}])
        self.assertTrue(transport.closed)

    def test_unary_pool_close_releases_and_reconnects(self) -> None:
        first = _SlowUnaryTransport()
        second = _SlowUnaryTransport()
        transports = [first, second]
        pool = UnaryDispatchPool(lambda: transports.pop(0))

        self.assertIs(pool.connected_transport(), first)
        pool.close()
        pool.close()
        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertEqual(result, {"ok": True})
        self.assertTrue(first.closed)
        self.assertFalse(second.closed)
        self.assertIs(pool.current_transport, second)

    def test_unary_pool_does_not_close_external_transport(self) -> None:
        transport = _SlowUnaryTransport(delay=0.02)
        pool = UnaryDispatchPool.from_transport(transport)

        with self.assertRaises(SDKError):
            pool.invoke(complete_draft().to_json_dict(), timeout=0.001)
        pool.close()
        _wait_until(lambda: len(transport.invocations) == 1)

        self.assertFalse(transport.closed)
        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
        self.assertEqual(result, {"ok": True})
        self.assertFalse(transport.closed)

    def test_stream_adapter_yields_values_until_terminal(self) -> None:
        stream = _FixedFrameStream(
            [_chunk("a"), _chunk("b"), _chunk("c"), _chunk(None, terminal=True)]
        )

        values = [item.value for item in StreamValueAdapter(stream)]

        self.assertEqual(values, ["a", "b", "c"])
        self.assertTrue(stream.closed)

    def test_stream_adapter_handles_empty_and_null_payloads(self) -> None:
        empty = _FixedFrameStream([_chunk(None, terminal=True)])
        null_value = _FixedFrameStream([_chunk(None), _chunk(None, terminal=True)])

        self.assertEqual([item.value for item in StreamValueAdapter(empty)], [])
        self.assertEqual(
            [item.value for item in StreamValueAdapter(null_value)],
            [None],
        )

    def test_stream_adapter_decodes_non_json_payload_bytes(self) -> None:
        stream = _FixedFrameStream(
            [
                {
                    "payload_json": None,
                    "payload_base64": "AAE=",
                    "content_type": "application/octet-stream",
                    "terminal": False,
                    "error": None,
                },
                _chunk(None, terminal=True),
            ]
        )

        values = [item.value for item in StreamValueAdapter(stream)]

        self.assertEqual(values, [b"\x00\x01"])

    def test_stream_adapter_idle_timeout_is_sdk_timeout(self) -> None:
        stream = _TimeoutFrameStream()

        with self.assertRaises(SDKError) as caught:
            list(StreamValueAdapter(stream, timeout=0.01))

        self.assertTrue(is_code(caught.exception, ErrorCode.TIMEOUT))
        self.assertEqual(caught.exception.details["reason"], "client_wait_timeout")
        self.assertTrue(stream.closed)

    def test_stream_adapter_projects_envelope_errors(self) -> None:
        stream = _FixedFrameStream(
            [
                _chunk("a"),
                _chunk(None, error={"kind": "UNAVAILABLE", "message": "down"}),
            ]
        )
        values: list[object] = []

        with self.assertRaises(SDKError) as caught:
            for item in StreamValueAdapter(stream):
                values.append(item.value)

        self.assertEqual(values, ["a"])
        self.assertTrue(is_code(caught.exception, ErrorCode.DAEMON_OFFLINE))
        self.assertEqual(caught.exception.message, "down")

    def test_stream_adapter_projects_host_error_payloads(self) -> None:
        stream = _FixedFrameStream(
            [
                _chunk(0),
                _chunk(1),
                _chunk(
                    {
                        "error": {
                            "kind": "INTERNAL",
                            "reason": "function_raised",
                            "message": "boom",
                        }
                    }
                ),
            ]
        )
        values: list[object] = []

        with self.assertRaises(SDKError) as caught:
            for item in StreamValueAdapter(stream):
                values.append(item.value)

        self.assertEqual(values, [0, 1])
        self.assertEqual(caught.exception.code, ErrorCode.ABILITY_FAILED)
        self.assertEqual(caught.exception.details["reason"], "function_raised")

    def test_stream_adapter_preserves_error_shaped_user_data(self) -> None:
        payload = {"error": {"detail": "data only"}, "ok": True}
        stream = _FixedFrameStream([_chunk(payload), _chunk(None, terminal=True)])

        values = [item.value for item in StreamValueAdapter(stream)]

        self.assertEqual(values, [payload])


def _write_control_discovery(
    tmp: str, *, invocation_endpoint: str | None = None
) -> Path:
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


class _SlowUnaryTransport:
    def __init__(self, *, delay: float = 0.0) -> None:
        self.delay = delay
        self.closed = False
        self.started = threading.Event()
        self.invocations: list[object] = []
        self.signed_signers: list[object] = []

    def invoke(self, invocation):
        self.started.set()
        if self.delay:
            time.sleep(self.delay)
        self.invocations.append(invocation)
        return {"ok": True}

    def invoke_signed(self, invocation, *, signer=None, options=None):
        self.started.set()
        if self.delay:
            time.sleep(self.delay)
        self.invocations.append(invocation)
        self.signed_signers.append(signer)
        return {"ok": True}

    def close(self) -> None:
        self.closed = True


class _MemoryBidiChannel:
    def __init__(
        self,
        *,
        frames: list[dict[str, object]] | None = None,
        close_requires_terminal: bool = False,
        close_error: str = "",
        timeout: bool = False,
    ) -> None:
        self.frames = list(frames or [])
        self.close_requires_terminal = close_requires_terminal
        self.close_error = close_error
        self.timeout = timeout
        self.sent: list[dict[str, object]] = []
        self.cancel_reasons: list[str] = []
        self.close_calls = 0
        self.closed = False
        self.terminal = False

    def send(self, frame: Mapping[str, object]) -> object:
        self.sent.append(dict(frame))
        return None

    def recv(self, timeout: float | None = None) -> Mapping[str, object] | None:
        if self.timeout:
            raise TimeoutError("no frame")
        if not self.frames:
            self.terminal = True
            return None
        frame = self.frames.pop(0)
        if frame.get("terminal") is True:
            self.terminal = True
        return frame

    def close(self) -> None:
        self.close_calls += 1
        if self.close_error:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="bidi",
                retry=RetryHint.NEVER,
                retryable=False,
                message=self.close_error,
            )
        if self.close_requires_terminal and not self.terminal:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="bidi",
                retry=RetryHint.NEVER,
                retryable=False,
                message="bidi session must be terminal before close",
            )
        self.closed = True

    def cancel(self, reason: str = "") -> object:
        self.cancel_reasons.append(reason)
        self.terminal = True
        return None


class _FixedFrameStream:
    def __init__(self, frames: list[dict[str, object]]) -> None:
        self._frames = frames
        self.closed = False

    def __iter__(self):
        return iter(self._frames)

    def close(self) -> None:
        self.closed = True


class _TimeoutFrameStream:
    def __init__(self) -> None:
        self.closed = False

    def recv(self, timeout: float | None = None):
        raise TimeoutError("blocked")

    def close(self) -> None:
        self.closed = True


def _chunk(
    payload_json: object = None,
    *,
    terminal: bool = False,
    error: object = None,
) -> dict[str, object]:
    return {
        "payload_json": payload_json,
        "payload_base64": (
            "bnVsbA==" if payload_json is None and not terminal else None
        ),
        "content_type": "application/json",
        "terminal": terminal,
        "error": error,
    }


def _wait_until(predicate) -> None:
    deadline = time.perf_counter() + 1.0
    while not predicate() and time.perf_counter() < deadline:
        time.sleep(0.01)


if __name__ == "__main__":
    unittest.main()
