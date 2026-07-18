import json
import queue
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
    ErrorClass,
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
from easynet_sdk.connection import ConnectOptions, RuntimeConnection

from test_cabi import FakeRawCABI
from test_runtime import (
    MemoryRuntimeTransport,
    canonical_runtime_receipt_pair,
    complete_draft,
)
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
                        "elapsed_ms": 12,
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
        self.assertEqual(result["elapsed_ms"], 12)
        self.assertEqual(result["admission_receipt"]["invocation_id"], "inv-runtime-1")
        self.assertEqual(result["terminal_receipt"]["invocation_id"], "inv-runtime-1")
        self.assertIn("authority_proof", result["terminal_receipt"])
        self.assertEqual(result["sdk_runtime_result"]["terminal_state"], "Completed")

    def test_invocation_result_adapter_context_manager_uses_transport_lifecycle(
        self,
    ) -> None:
        runtime = MemoryRuntimeTransport()
        adapter = InvocationResultAdapter.from_runtime_client(RuntimeClient(runtime))

        with adapter as entered:
            self.assertIs(entered, adapter)
            self.assertTrue(adapter.invoke(complete_draft())["ok"])
            self.assertEqual(runtime.close_calls, 0)

        self.assertEqual(runtime.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            adapter.__enter__()
        self.assertTrue(is_code(caught.exception, ErrorCode.CANCELLED))

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
                result["terminal_receipt"]["receipt_ura"] = (
                    "easynet:///r/example/resource/agent.alice.sdk/"
                    "invocation/opaque/receipt"
                )
                result["terminal_receipt"]["self_hash_hex"] = "55" * 32
                return json.dumps(result, separators=(",", ":"), sort_keys=True).encode(
                    "utf-8"
                )

        runtime = ReceiptRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(
            RuntimeClient(runtime)
        )

        result = transport.invoke(complete_draft().to_json_dict())

        self.assertNotIn("receipt", result)
        self.assertNotIn("receipt_summary", result)
        self.assertEqual(result["terminal_receipt"]["invocation_id"], "inv-runtime-1")
        self.assertEqual(
            result["terminal_receipt_summary"]["invocation_id"], "inv-runtime-1"
        )
        self.assertTrue(result["terminal_receipt_summary"]["has_causal_anchor"])

    def test_invocation_result_adapter_raises_on_non_ok_runtime_result(self) -> None:
        class FailedRuntimeTransport(MemoryRuntimeTransport):
            def invoke(self, draft_json: bytes) -> bytes:
                draft = json.loads(draft_json.decode("utf-8"))
                admission, terminal = canonical_runtime_receipt_pair(
                    "inv-failed", "Failed"
                )
                return json.dumps(
                    {
                        "ok": False,
                        "tuple": draft,
                        "invocation_id": "inv-failed",
                        "terminal_state": "Failed",
                        "admission_receipt": admission,
                        "terminal_receipt": terminal,
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

    def test_invocation_result_adapter_preserves_extension_failure_code(self) -> None:
        class FailedRuntimeTransport(MemoryRuntimeTransport):
            def invoke(self, draft_json: bytes) -> bytes:
                draft = json.loads(draft_json.decode("utf-8"))
                admission, terminal = canonical_runtime_receipt_pair(
                    "inv-membership", "Failed"
                )
                return json.dumps(
                    {
                        "ok": False,
                        "tuple": draft,
                        "invocation_id": "inv-membership",
                        "terminal_state": "Failed",
                        "admission_receipt": admission,
                        "terminal_receipt": terminal,
                        "error": {
                            "code": "AXON_MEMBERSHIP_REQUIRED",
                            "stage": "runtime",
                            "message": "membership required",
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

        self.assertEqual(caught.exception.code, "AXON_MEMBERSHIP_REQUIRED")
        self.assertEqual(caught.exception.error_class, ErrorClass.GENERIC)
        self.assertEqual(caught.exception.message, "membership required")

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

    def test_daemon_transport_tracks_real_runtime_close_failure(
        self,
    ) -> None:
        raw = MemoryRuntimeTransport()
        raw.close_error = RuntimeError("runtime close failed")
        runtime = RuntimeClient(raw)
        transport = DaemonInvocationTransport.from_runtime_client(runtime)

        with self.assertRaises(SDKError) as first:
            transport.close()

        self.assertFalse(transport._closed)
        self.assertEqual(raw.close_calls, 1)

        with self.assertRaises(SDKError) as second:
            transport.close()

        self.assertIs(second.exception, first.exception)
        self.assertFalse(transport._closed)
        self.assertEqual(raw.close_calls, 1)

    def test_daemon_transport_tracks_real_connection_close_failure(self) -> None:
        connector = _CloseFailsRuntimeConnector()
        connection = RuntimeConnection(connector)
        connection.connect(ConnectOptions(endpoint="unix:///daemon.sock"))
        transport = DaemonInvocationTransport(
            runtime=connection.runtime_client(),
            connection=connection,
        )

        with self.assertRaises(SDKError) as first:
            transport.close()
        with self.assertRaises(SDKError) as second:
            transport.close()

        self.assertIs(second.exception, first.exception)
        self.assertEqual(connector.close_calls, 1)
        self.assertFalse(transport._closed)

    def test_daemon_transport_close_waits_for_active_operations(self) -> None:
        signer = Signer.from_signature(
            signer_handle(),
            InvocationSignature(
                algorithm="ed25519",
                signature_base64="c2lnbmF0dXJl",
            ),
        )

        for operation_name, raw_method in (
            ("invoke", "invoke"),
            ("invoke_signed", "prepare"),
            ("stream", "open_stream"),
            ("bidi", "open_bidi"),
        ):
            with self.subTest(operation=operation_name):
                entered = threading.Event()
                release = threading.Event()
                raw = MemoryRuntimeTransport()
                runtime = RuntimeClient(raw)
                transport = DaemonInvocationTransport.from_runtime_client(runtime)
                original = getattr(raw, raw_method)
                operation_errors: list[BaseException] = []
                close_errors: list[BaseException] = []

                def blocked(*args, **kwargs):
                    entered.set()
                    release.wait()
                    return original(*args, **kwargs)

                def operate() -> None:
                    try:
                        if operation_name == "invoke":
                            transport.invoke(complete_draft())
                        elif operation_name == "invoke_signed":
                            transport.invoke_signed(complete_draft(), signer=signer)
                        elif operation_name == "stream":
                            transport.stream(complete_draft())
                        else:
                            transport.bidi(complete_draft())
                    except BaseException as exc:
                        operation_errors.append(exc)

                def close() -> None:
                    try:
                        transport.close()
                    except BaseException as exc:
                        close_errors.append(exc)

                with patch.object(raw, raw_method, side_effect=blocked):
                    operation = threading.Thread(target=operate, daemon=True)
                    closer = threading.Thread(target=close, daemon=True)
                    operation.start()
                    self.assertTrue(entered.wait(timeout=1.0))
                    closer.start()
                    _wait_until(lambda: transport._state.value == "closing")
                    self.assertTrue(closer.is_alive())
                    self.assertEqual(raw.close_calls, 0)
                    release.set()
                    operation.join(timeout=1.0)
                    closer.join(timeout=1.0)

                self.assertFalse(operation.is_alive())
                self.assertFalse(closer.is_alive())
                self.assertEqual(operation_errors, [])
                self.assertEqual(close_errors, [])
                self.assertEqual(raw.close_calls, 1)

    def test_invoke_signed_retains_failed_handle_cleanup_until_close(self) -> None:
        raw = _HandleCleanupFailsOnceRuntimeTransport()
        transport = DaemonInvocationTransport.from_runtime_client(RuntimeClient(raw))
        signer = Signer.from_signature(
            signer_handle(),
            InvocationSignature(
                algorithm="ed25519",
                signature_base64="c2lnbmF0dXJl",
            ),
        )

        with self.assertRaises(SDKError):
            transport.invoke_signed(complete_draft(), signer=signer)

        self.assertEqual(raw.free_handle_calls, 1)
        self.assertEqual(raw.close_calls, 0)

        transport.close()

        self.assertEqual(raw.free_handle_calls, 2)
        self.assertEqual(raw.close_calls, 1)
        self.assertTrue(transport._closed)

    def test_connect_cleans_up_partially_acquired_connection(self) -> None:
        acquisition_error = RuntimeError("handshake failed")
        connection = _PartialConnection(acquisition_error)

        with (
            patch(
                "easynet_sdk._cabi.open_cabi_runtime_connector",
                return_value=object(),
            ),
            patch(
                "easynet_sdk.providers.easynet.transport.RuntimeConnection",
                return_value=connection,
            ),
            self.assertRaises(RuntimeError) as caught,
        ):
            DaemonInvocationTransport.connect()

        self.assertIs(caught.exception, acquisition_error)
        self.assertEqual(connection.close_calls, 1)

    def test_connect_cleanup_failure_preserves_acquisition_error(self) -> None:
        acquisition_error = RuntimeError("handshake failed")
        cleanup_error = RuntimeError("connection cleanup failed")
        connection = _PartialConnection(acquisition_error, cleanup_error)

        with (
            patch(
                "easynet_sdk._cabi.open_cabi_runtime_connector",
                return_value=object(),
            ),
            patch(
                "easynet_sdk.providers.easynet.transport.RuntimeConnection",
                return_value=connection,
            ),
            self.assertRaises(RuntimeError) as caught,
        ):
            DaemonInvocationTransport.connect()

        self.assertIs(caught.exception, acquisition_error)
        self.assertIs(caught.exception.__cause__, cleanup_error)
        self.assertEqual(connection.close_calls, 1)

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
        channel.close()
        cancelled_channel = transport.bidi(
            complete_draft().to_json_dict(),
            [{"stream_id": 1, "content_type": "application/json"}],
        )
        cancelled = cancelled_channel.cancel("client stop")
        cancelled_channel.close()

        self.assertEqual(ack["sequence"], 1)
        self.assertEqual(half_closed["state"], "HalfClosedLocal")
        self.assertFalse(half_closed["terminal"])
        self.assertEqual(channel.session.state, BidiState.CLOSED)
        self.assertEqual(cancelled["state"], "CancelRequested")
        self.assertEqual(cancelled_channel.session.state, BidiState.CLOSED)

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

    def test_bidi_close_releases_open_session_without_claiming_cancellation(
        self,
    ) -> None:
        channel = _MemoryBidiChannel()
        session = BidiSessionAdapter(channel)

        session.close()
        session.close()

        self.assertEqual(channel.cancel_reasons, [])
        self.assertEqual(channel.close_calls, 1)
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
        channel = _MemoryBidiChannel(
            frames=[
                {
                    "sequence": 1,
                    "kind": "terminal",
                    "terminal": True,
                    "terminal_receipt": {
                        "receipt_ura": (
                            "easynet:///r/test/resource/agent.test.sdk/"
                            "invocation/invocation-1/receipt"
                        )
                    },
                }
            ]
        )
        session = BidiSessionAdapter(channel)

        session.cancel("user stop")
        terminal = session.recv()

        self.assertEqual(channel.cancel_reasons, ["user stop"])
        self.assertEqual(channel.close_calls, 0)
        self.assertFalse(channel.closed)
        self.assertIsNotNone(terminal)
        self.assertTrue(terminal["terminal"])

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
        release = threading.Event()
        first = _SlowUnaryTransport(release=release)
        second = _SlowUnaryTransport()
        transports = [first, second]
        pool = UnaryDispatchPool(lambda: transports.pop(0))
        errors: list[SDKError] = []

        def invoke_until_timeout() -> None:
            try:
                pool.invoke(complete_draft().to_json_dict(), timeout=0.01)
            except SDKError as exc:
                errors.append(exc)

        caller = threading.Thread(target=invoke_until_timeout, daemon=True)
        caller.start()
        self.assertTrue(first.started.wait(timeout=1.0))
        caller.join(timeout=1.0)

        self.assertFalse(caller.is_alive())
        self.assertEqual(len(errors), 1)
        caught = errors[0]
        self.assertTrue(is_code(caught, ErrorCode.TIMEOUT))
        self.assertEqual(caught.details["reason"], "client_wait_timeout")
        self.assertEqual(caught.retry, RetryHint.UNKNOWN)
        self.assertFalse(caught.retryable)
        self.assertEqual(caught.details["execution_state"], "unknown")
        self.assertIsNone(pool.current_transport)

        release.set()
        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertTrue(result["ok"])
        self.assertEqual(transports, [])
        self.assertIs(pool.current_transport, second)
        _wait_until(lambda: first.closed)
        self.assertEqual(len(first.invocations), 1)
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

    def test_unary_pool_result_wait_uses_remaining_budget(self) -> None:
        release = threading.Event()
        transport = _SlowUnaryTransport(release=release)
        pool = UnaryDispatchPool(lambda: transport)
        original_start = threading.Thread.start

        def start_after_dispatch(worker: threading.Thread) -> None:
            original_start(worker)
            self.assertTrue(transport.started.wait(timeout=1.0))

        try:
            with (
                patch(
                    "easynet_sdk.transport.time.monotonic",
                    side_effect=[0.0, 0.0, 0.99],
                ),
                patch(
                    "easynet_sdk.transport.threading.Thread.start",
                    new=start_after_dispatch,
                ),
                self.assertRaises(SDKError) as caught,
            ):
                pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

            self.assertEqual(caught.exception.retry, RetryHint.UNKNOWN)
            self.assertFalse(caught.exception.retryable)
        finally:
            release.set()

        _wait_until(lambda: transport.closed)
        self.assertTrue(transport.closed)

    def test_unary_pool_timeout_waits_to_cleanup_until_outcome_publication(
        self,
    ) -> None:
        publication_started = threading.Event()
        release_publication = threading.Event()
        publication_completed = threading.Event()

        class OutcomePublicationQueue:
            def __init__(self, maxsize: int) -> None:
                self.item = None

            def put(self, item) -> None:
                publication_started.set()
                release_publication.wait()
                self.item = item
                publication_completed.set()

            def get(self, timeout: float | None = None):
                self.assert_publication_started()
                raise queue.Empty

            @staticmethod
            def assert_publication_started() -> None:
                if not publication_started.wait(timeout=1.0):
                    raise AssertionError("worker did not reach outcome publication")

        transport = _SlowUnaryTransport()
        pool = UnaryDispatchPool(lambda: transport)

        with (
            patch("easynet_sdk.transport.queue.Queue", OutcomePublicationQueue),
            self.assertRaises(SDKError) as caught,
        ):
            pool.invoke(complete_draft().to_json_dict(), timeout=0.0)

        self.assertEqual(caught.exception.retry, RetryHint.UNKNOWN)
        self.assertIsNone(pool.current_transport)
        self.assertFalse(transport.closed)

        release_publication.set()
        self.assertTrue(publication_completed.wait(timeout=1.0))
        _wait_until(lambda: transport.closed)

    def test_unary_pool_waiters_do_not_spawn_behind_stuck_flight(self) -> None:
        release = threading.Event()
        transport = _SlowUnaryTransport(release=release)
        factory_calls = 0

        def factory() -> _SlowUnaryTransport:
            nonlocal factory_calls
            factory_calls += 1
            return transport

        pool = UnaryDispatchPool(factory)
        baseline_workers = _sdk_unary_worker_count()
        first_errors: list[SDKError] = []

        def invoke_stuck_flight() -> None:
            try:
                pool.invoke(complete_draft().to_json_dict(), timeout=0.1)
            except SDKError as exc:
                first_errors.append(exc)

        caller = threading.Thread(target=invoke_stuck_flight, daemon=True)
        try:
            caller.start()
            self.assertTrue(transport.started.wait(timeout=1.0))
            caller.join(timeout=1.0)
            self.assertFalse(caller.is_alive())
            self.assertEqual(len(first_errors), 1)
            self.assertEqual(first_errors[0].retry, RetryHint.UNKNOWN)

            workers_during_stuck_flight = _sdk_unary_worker_count()
            self.assertEqual(workers_during_stuck_flight, baseline_workers + 1)

            for _ in range(8):
                with self.assertRaises(SDKError) as caught:
                    pool.invoke(complete_draft().to_json_dict(), timeout=0.005)
                self.assertEqual(caught.exception.retry, RetryHint.SAFE)
                self.assertEqual(
                    caught.exception.details["execution_state"], "not_started"
                )

            self.assertEqual(factory_calls, 1)
            self.assertEqual(_sdk_unary_worker_count(), workers_during_stuck_flight)
        finally:
            release.set()

        _wait_until(lambda: len(transport.invocations) == 1)
        _wait_until(lambda: _sdk_unary_worker_count() == baseline_workers)
        self.assertEqual(len(transport.invocations), 1)
        self.assertEqual(_sdk_unary_worker_count(), baseline_workers)

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
        self.assertEqual(caught.exception.retry, RetryHint.SAFE)
        self.assertTrue(caught.exception.retryable)
        self.assertEqual(caught.exception.details["execution_state"], "not_started")
        self.assertEqual(result, [{"ok": True}])
        self.assertEqual(len(first.invocations), 1)
        self.assertFalse(first.closed)
        self.assertIs(pool.current_transport, first)
        self.assertEqual(transports, [second])

    def test_unary_pool_quiesce_cancels_pre_barrier_queued_waiter(self) -> None:
        release = threading.Event()
        first = _SlowUnaryTransport(release=release)
        second = _SlowUnaryTransport()
        transports = [first, second]
        flight_lock = _ObservedFlightLock()
        pool = UnaryDispatchPool(lambda: transports.pop(0))
        pool._flight_lock = flight_lock
        results: list[dict[str, object]] = []
        queued_errors: list[BaseException] = []

        active = threading.Thread(
            target=lambda: results.append(
                pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
            ),
            daemon=True,
        )

        def invoke_queued() -> None:
            try:
                pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
            except BaseException as exc:
                queued_errors.append(exc)

        queued = threading.Thread(target=invoke_queued, daemon=True)
        active.start()
        self.assertTrue(first.started.wait(timeout=1.0))
        queued.start()
        self.assertTrue(flight_lock.second_attempt.wait(timeout=1.0))

        pool.quiesce()
        release.set()
        active.join(timeout=1.0)
        queued.join(timeout=1.0)

        self.assertFalse(active.is_alive())
        self.assertFalse(queued.is_alive())
        self.assertEqual(results, [{"ok": True}])
        self.assertEqual(len(queued_errors), 1)
        self.assertTrue(is_code(queued_errors[0], ErrorCode.CANCELLED))
        self.assertEqual(transports, [second])
        _wait_until(lambda: first.closed)
        _wait_until(lambda: pool._state.value == "quiescent")

        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertEqual(result, {"ok": True})
        self.assertIs(pool.current_transport, second)

    def test_unary_pool_thread_start_failure_releases_flight(self) -> None:
        transport = _SlowUnaryTransport()
        factory_calls = 0

        def factory() -> _SlowUnaryTransport:
            nonlocal factory_calls
            factory_calls += 1
            return transport

        pool = UnaryDispatchPool(factory)

        with patch(
            "easynet_sdk.transport.threading.Thread.start",
            side_effect=RuntimeError("thread start failed"),
        ):
            with self.assertRaisesRegex(RuntimeError, "thread start failed"):
                pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertEqual(result, {"ok": True})
        self.assertEqual(factory_calls, 1)

    def test_unary_pool_factory_exception_releases_flight(self) -> None:
        transport = _SlowUnaryTransport()
        factory_calls = 0

        def factory() -> _SlowUnaryTransport:
            nonlocal factory_calls
            factory_calls += 1
            if factory_calls == 1:
                raise RuntimeError("factory failed")
            return transport

        pool = UnaryDispatchPool(factory)

        with self.assertRaisesRegex(RuntimeError, "factory failed"):
            pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertEqual(result, {"ok": True})
        self.assertEqual(factory_calls, 2)

    def test_unary_pool_operation_exception_releases_flight(self) -> None:
        transport = _FailOnceUnaryTransport()
        pool = UnaryDispatchPool.from_transport(transport)

        with self.assertRaisesRegex(RuntimeError, "operation failed"):
            pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertEqual(result, {"ok": True})
        self.assertEqual(len(transport.invocations), 2)

    def test_unary_pool_quiesce_during_active_invoke_is_bounded(self) -> None:
        release = threading.Event()
        first = _SlowUnaryTransport(release=release)
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
        started = time.perf_counter()
        pool.quiesce()
        elapsed = time.perf_counter() - started

        try:
            self.assertLess(elapsed, 0.02)
            self.assertIsNone(pool.current_transport)
        finally:
            release.set()
        thread.join(timeout=1.0)
        self.assertFalse(thread.is_alive())
        self.assertEqual(result, [{"ok": True}])
        _wait_until(lambda: first.closed)
        _wait_until(lambda: pool._state.value == "quiescent")
        self.assertTrue(first.closed)

        reopened = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertEqual(reopened, {"ok": True})
        self.assertIs(pool.current_transport, second)

    def test_unary_pool_blocking_cleanup_releases_result_and_flight(self) -> None:
        operation_release = threading.Event()
        cleanup_release = threading.Event()
        first = _BlockingCloseUnaryTransport(
            release=operation_release,
            close_release=cleanup_release,
        )
        second = _SlowUnaryTransport()
        transports = [first, second]
        pool = UnaryDispatchPool(lambda: transports.pop(0))
        results: list[dict[str, object]] = []
        baseline_operation_workers = _sdk_unary_worker_count()
        baseline_cleanup_workers = _sdk_unary_cleanup_worker_count()
        caller = threading.Thread(
            target=lambda: results.append(
                pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
            ),
            daemon=True,
        )

        caller.start()
        self.assertTrue(first.started.wait(timeout=1.0))
        pool.quiesce()
        operation_release.set()
        caller.join(timeout=0.5)

        self.assertFalse(caller.is_alive())
        self.assertEqual(results, [{"ok": True}])
        self.assertTrue(first.close_started.wait(timeout=1.0))
        _wait_until(lambda: _sdk_unary_worker_count() == baseline_operation_workers)
        self.assertEqual(
            _sdk_unary_cleanup_worker_count(), baseline_cleanup_workers + 1
        )
        self.assertTrue(pool._flight_lock.acquire(blocking=False))
        pool._flight_lock.release()

        cleanup_release.set()
        _wait_until(lambda: first.closed)
        _wait_until(
            lambda: _sdk_unary_cleanup_worker_count() == baseline_cleanup_workers
        )
        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
        self.assertEqual(result, {"ok": True})
        self.assertIs(pool.current_transport, second)

    def test_unary_pool_idle_blocking_close_does_not_hold_flight(self) -> None:
        close_release = threading.Event()
        first = _BlockingCloseUnaryTransport(
            release=threading.Event(),
            close_release=close_release,
        )
        second = _SlowUnaryTransport()
        transports = [first, second]
        pool = UnaryDispatchPool(lambda: transports.pop(0))
        close_errors: list[BaseException] = []
        self.assertIs(pool.connected_transport(), first)

        def close() -> None:
            try:
                pool.quiesce()
            except BaseException as exc:
                close_errors.append(exc)

        closer = threading.Thread(target=close, daemon=True)
        closer.start()
        self.assertTrue(first.close_started.wait(timeout=1.0))

        self.assertTrue(pool._flight_lock.acquire(blocking=False))
        pool._flight_lock.release()

        close_release.set()
        closer.join(timeout=1.0)
        self.assertFalse(closer.is_alive())
        self.assertEqual(close_errors, [])
        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
        self.assertEqual(result, {"ok": True})
        self.assertIs(pool.current_transport, second)

    def test_unary_pool_concurrent_close_waits_for_same_success(self) -> None:
        close_release = threading.Event()
        transport = _BlockingCloseUnaryTransport(
            release=threading.Event(),
            close_release=close_release,
        )
        pool = UnaryDispatchPool(lambda: transport)
        self.assertIs(pool.connected_transport(), transport)
        completed: list[str] = []
        errors: list[BaseException] = []

        def close(name: str) -> None:
            try:
                pool.quiesce()
                completed.append(name)
            except BaseException as exc:
                errors.append(exc)

        first = threading.Thread(target=close, args=("first",), daemon=True)
        second = threading.Thread(target=close, args=("second",), daemon=True)
        first.start()
        self.assertTrue(transport.close_started.wait(timeout=1.0))
        second.start()

        self.assertFalse(_wait_for_thread(second, timeout=0.05))
        self.assertEqual(transport.close_calls, 1)
        close_release.set()
        first.join(timeout=1.0)
        second.join(timeout=1.0)

        self.assertEqual(errors, [])
        self.assertCountEqual(completed, ["first", "second"])
        self.assertEqual(transport.close_calls, 1)

    def test_unary_pool_concurrent_close_replays_same_failure(self) -> None:
        close_release = threading.Event()
        close_error = RuntimeError("delegated close failed")
        transport = _BlockingCloseUnaryTransport(
            release=threading.Event(),
            close_release=close_release,
            close_error=close_error,
        )
        pool = UnaryDispatchPool(lambda: transport)
        self.assertIs(pool.connected_transport(), transport)
        errors: list[BaseException] = []

        def close() -> None:
            try:
                pool.quiesce()
            except BaseException as exc:
                errors.append(exc)

        first = threading.Thread(target=close, daemon=True)
        second = threading.Thread(target=close, daemon=True)
        first.start()
        self.assertTrue(transport.close_started.wait(timeout=1.0))
        second.start()

        self.assertFalse(_wait_for_thread(second, timeout=0.05))
        self.assertEqual(transport.close_calls, 1)
        close_release.set()
        first.join(timeout=1.0)
        second.join(timeout=1.0)

        self.assertEqual(len(errors), 2)
        self.assertIs(errors[0], close_error)
        self.assertIs(errors[1], close_error)
        self.assertEqual(transport.close_calls, 1)

    def test_unary_pool_quiesce_releases_and_reconnects(self) -> None:
        first = _SlowUnaryTransport()
        second = _SlowUnaryTransport()
        transports = [first, second]
        pool = UnaryDispatchPool(lambda: transports.pop(0))

        self.assertIs(pool.connected_transport(), first)
        pool.quiesce()
        pool.quiesce()
        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertEqual(result, {"ok": True})
        self.assertTrue(first.closed)
        self.assertFalse(second.closed)
        self.assertIs(pool.current_transport, second)
        self.assertEqual(transports, [])

    def test_unary_pool_close_is_terminal(self) -> None:
        transport = _SlowUnaryTransport()
        pool = UnaryDispatchPool(lambda: transport)
        self.assertIs(pool.connected_transport(), transport)

        pool.close()
        pool.close()

        self.assertTrue(transport.closed)
        self.assertEqual(pool._state.value, "closed")
        with self.assertRaises(SDKError) as caught:
            pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
        self.assertTrue(is_code(caught.exception, ErrorCode.CANCELLED))

    def test_unary_pool_does_not_close_external_transport(self) -> None:
        transport = _SlowUnaryTransport(delay=0.02)
        pool = UnaryDispatchPool.from_transport(transport)

        with self.assertRaises(SDKError):
            pool.invoke(complete_draft().to_json_dict(), timeout=0.001)
        pool.quiesce()
        _wait_until(lambda: len(transport.invocations) == 1)

        self.assertFalse(transport.closed)
        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertEqual(result, {"ok": True})
        self.assertFalse(transport.closed)

    def test_unary_pool_quiesce_before_factory_publish_closes_candidate(self) -> None:
        candidate = _SlowUnaryTransport()
        reopened = _SlowUnaryTransport()
        factory_entered = threading.Event()
        release_factory = threading.Event()
        errors: list[BaseException] = []
        factory_calls = 0

        def factory() -> _SlowUnaryTransport:
            nonlocal factory_calls
            factory_calls += 1
            if factory_calls > 1:
                return reopened
            factory_entered.set()
            release_factory.wait()
            return candidate

        pool = UnaryDispatchPool(factory)

        def invoke() -> None:
            try:
                pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
            except BaseException as exc:
                errors.append(exc)

        caller = threading.Thread(target=invoke, daemon=True)
        caller.start()
        self.assertTrue(factory_entered.wait(timeout=1.0))
        pool.quiesce()
        self.assertIsNone(pool.current_transport)
        release_factory.set()
        caller.join(timeout=1.0)

        self.assertFalse(caller.is_alive())
        self.assertEqual(len(errors), 1)
        self.assertTrue(is_code(errors[0], ErrorCode.CANCELLED))
        _wait_until(lambda: candidate.closed)
        _wait_until(lambda: pool._state.value == "quiescent")
        self.assertTrue(candidate.closed)
        self.assertIsNone(pool.current_transport)

        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertEqual(result, {"ok": True})
        self.assertIs(pool.current_transport, reopened)

    def test_unary_pool_quiesce_before_worker_factory_call_cancels_generation(
        self,
    ) -> None:
        transport = _SlowUnaryTransport()
        worker_ready = threading.Event()
        release_worker = threading.Event()
        errors: list[BaseException] = []
        factory_calls = 0
        original_start = threading.Thread.start

        def factory() -> _SlowUnaryTransport:
            nonlocal factory_calls
            factory_calls += 1
            return transport

        def delayed_worker_start(worker: threading.Thread) -> None:
            if worker.name != "easynet-sdk-unary":
                original_start(worker)
                return
            worker_ready.set()
            release_worker.wait()
            original_start(worker)

        pool = UnaryDispatchPool(factory)

        def invoke() -> None:
            try:
                pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
            except BaseException as exc:
                errors.append(exc)

        caller = threading.Thread(target=invoke, daemon=True)
        with patch(
            "easynet_sdk.transport.threading.Thread.start",
            new=delayed_worker_start,
        ):
            caller.start()
            self.assertTrue(worker_ready.wait(timeout=1.0))
            pool.quiesce()
            release_worker.set()
            caller.join(timeout=1.0)

        self.assertFalse(caller.is_alive())
        self.assertEqual(len(errors), 1)
        self.assertTrue(is_code(errors[0], ErrorCode.CANCELLED))
        self.assertEqual(factory_calls, 0)
        self.assertIsNone(pool.current_transport)

        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertEqual(result, {"ok": True})
        self.assertEqual(factory_calls, 1)
        self.assertIs(pool.current_transport, transport)

    def test_unary_pool_quiesce_is_bounded_while_factory_is_blocked(self) -> None:
        candidate = _SlowUnaryTransport()
        reopened = _SlowUnaryTransport()
        factory_entered = threading.Event()
        release_factory = threading.Event()
        errors: list[BaseException] = []
        factory_calls = 0

        def factory() -> _SlowUnaryTransport:
            nonlocal factory_calls
            factory_calls += 1
            if factory_calls > 1:
                return reopened
            factory_entered.set()
            release_factory.wait()
            return candidate

        pool = UnaryDispatchPool(factory)

        def connect() -> None:
            try:
                pool.connected_transport()
            except BaseException as exc:
                errors.append(exc)

        caller = threading.Thread(target=connect, daemon=True)
        caller.start()
        self.assertTrue(factory_entered.wait(timeout=1.0))

        started = time.perf_counter()
        pool.quiesce()
        elapsed = time.perf_counter() - started

        try:
            self.assertLess(elapsed, 0.02)
            self.assertIsNone(pool.current_transport)
        finally:
            release_factory.set()
        caller.join(timeout=1.0)
        self.assertFalse(caller.is_alive())
        self.assertEqual(len(errors), 1)
        self.assertTrue(is_code(errors[0], ErrorCode.CANCELLED))
        _wait_until(lambda: candidate.closed)
        _wait_until(lambda: pool._state.value == "quiescent")
        self.assertTrue(candidate.closed)

        self.assertIs(pool.connected_transport(), reopened)

    def test_unary_pool_concurrent_publication_closes_duplicate(self) -> None:
        first = _SlowUnaryTransport()
        second = _SlowUnaryTransport()
        candidates = [first, second]
        factory_lock = threading.Lock()
        publish_barrier = threading.Barrier(3)
        results: list[_SlowUnaryTransport] = []
        errors: list[BaseException] = []

        def factory() -> _SlowUnaryTransport:
            with factory_lock:
                candidate = candidates.pop()
            publish_barrier.wait(timeout=1.0)
            return candidate

        pool = UnaryDispatchPool(factory)

        def connect() -> None:
            try:
                results.append(pool.connected_transport())
            except BaseException as exc:
                errors.append(exc)

        callers = [threading.Thread(target=connect, daemon=True) for _ in range(2)]
        for caller in callers:
            caller.start()
        publish_barrier.wait(timeout=1.0)
        for caller in callers:
            caller.join(timeout=1.0)

        self.assertEqual(errors, [])
        self.assertEqual(len(results), 2)
        self.assertIs(results[0], results[1])
        winner = results[0]
        loser = second if winner is first else first
        self.assertFalse(winner.closed)
        _wait_until(lambda: loser.closed)
        self.assertTrue(loser.closed)
        self.assertIs(pool.current_transport, winner)

        pool.quiesce()
        self.assertTrue(winner.closed)

    def test_unary_pool_quiesce_failure_is_retained_and_retried(self) -> None:
        first = _CloseFailsOnceUnaryTransport()
        second = _SlowUnaryTransport()
        transports = [first, second]
        pool = UnaryDispatchPool(lambda: transports.pop(0))
        self.assertIs(pool.connected_transport(), first)

        with self.assertRaisesRegex(RuntimeError, "close failed"):
            pool.quiesce()

        self.assertIsNone(pool.current_transport)
        self.assertEqual(first.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
        self.assertTrue(is_code(caught.exception, ErrorCode.CANCELLED))

        pool.quiesce()
        self.assertEqual(first.close_calls, 2)
        self.assertTrue(first.closed)

        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertEqual(result, {"ok": True})
        self.assertIs(pool.current_transport, second)

    def test_unary_pool_quiesce_aggregates_recorded_cleanup_failures(self) -> None:
        first = _CloseFailsOnceUnaryTransport()
        second = _CloseFailsOnceUnaryTransport()
        candidates = [first, second]
        factory_lock = threading.Lock()
        publish_barrier = threading.Barrier(3)
        errors: list[BaseException] = []

        def factory() -> _CloseFailsOnceUnaryTransport:
            with factory_lock:
                candidate = candidates.pop()
            publish_barrier.wait(timeout=1.0)
            return candidate

        pool = UnaryDispatchPool(factory)

        def connect() -> None:
            try:
                pool.connected_transport()
            except BaseException as exc:
                errors.append(exc)

        callers = [threading.Thread(target=connect, daemon=True) for _ in range(2)]
        for caller in callers:
            caller.start()
        publish_barrier.wait(timeout=1.0)
        for caller in callers:
            caller.join(timeout=1.0)

        self.assertLessEqual(len(errors), 1)
        if errors:
            self.assertTrue(is_code(errors[0], ErrorCode.CANCELLED))
        _wait_until(lambda: first.close_calls + second.close_calls == 1)
        _wait_until(lambda: _sdk_unary_cleanup_worker_count() == 0)

        with self.assertRaises(BaseExceptionGroup) as caught:
            pool.quiesce()

        self.assertEqual(len(caught.exception.exceptions), 2)
        self.assertEqual(
            [str(exc) for exc in caught.exception.exceptions],
            ["close failed", "close failed"],
        )
        pool.quiesce()
        self.assertTrue(first.closed)
        self.assertTrue(second.closed)

    def test_unary_pool_operation_error_is_not_replaced_by_close_error(self) -> None:
        release = threading.Event()
        first = _OperationAndCloseFailTransport(release=release)
        second = _SlowUnaryTransport()
        transports = [first, second]
        pool = UnaryDispatchPool(lambda: transports.pop(0))
        operation_errors: list[BaseException] = []

        def invoke() -> None:
            try:
                pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
            except BaseException as exc:
                operation_errors.append(exc)

        caller = threading.Thread(target=invoke, daemon=True)
        caller.start()
        self.assertTrue(first.started.wait(timeout=1.0))
        pool.quiesce()
        release.set()
        caller.join(timeout=1.0)

        self.assertFalse(caller.is_alive())
        self.assertEqual(len(operation_errors), 1)
        self.assertEqual(str(operation_errors[0]), "operation failed")
        _wait_until(lambda: first.close_calls == 1)
        _wait_until(lambda: _sdk_unary_cleanup_worker_count() == 0)
        self.assertEqual(first.close_calls, 1)
        self.assertFalse(first.closed)

        with self.assertRaisesRegex(RuntimeError, "close failed"):
            pool.quiesce()

        self.assertEqual(first.close_calls, 2)
        self.assertTrue(first.closed)
        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)
        self.assertEqual(result, {"ok": True})
        self.assertIs(pool.current_transport, second)

    def test_unary_pool_timeout_close_failure_remains_owned(self) -> None:
        release = threading.Event()
        transport = _CloseFailsOnceUnaryTransport(release=release)
        reopened = _SlowUnaryTransport()
        transports = [transport, reopened]
        pool = UnaryDispatchPool(lambda: transports.pop(0))
        errors: list[SDKError] = []

        def invoke_until_timeout() -> None:
            try:
                pool.invoke(complete_draft().to_json_dict(), timeout=0.1)
            except SDKError as exc:
                errors.append(exc)

        caller = threading.Thread(target=invoke_until_timeout, daemon=True)
        caller.start()
        self.assertTrue(transport.started.wait(timeout=1.0))
        caller.join(timeout=1.0)
        self.assertEqual(len(errors), 1)
        self.assertEqual(errors[0].retry, RetryHint.UNKNOWN)

        release.set()
        _wait_until(lambda: transport.close_calls == 1)
        _wait_until(lambda: _sdk_unary_cleanup_worker_count() == 0)
        self.assertEqual(transport.close_calls, 1)
        self.assertFalse(transport.closed)

        with self.assertRaisesRegex(RuntimeError, "close failed"):
            pool.quiesce()
        self.assertEqual(transport.close_calls, 2)
        self.assertTrue(transport.closed)

        result = pool.invoke(complete_draft().to_json_dict(), timeout=1.0)

        self.assertEqual(result, {"ok": True})
        self.assertIs(pool.current_transport, reopened)

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


class _CloseFailsRuntimeConnector:
    def __init__(self) -> None:
        self.close_calls = 0
        self.runtime = MemoryRuntimeTransport()

    def resolve(self, options_json: bytes) -> bytes:
        return b'{"endpoint":"unix:///daemon.sock"}'

    def handshake(self, endpoint_json: bytes):
        return self.runtime, b'{"ready":true}'

    def close(self) -> None:
        self.close_calls += 1
        raise RuntimeError("connection close failed")


class _HandleCleanupFailsOnceRuntimeTransport(MemoryRuntimeTransport):
    def __init__(self) -> None:
        super().__init__()
        self.free_handle_calls = 0

    def free_handle(self, control) -> None:
        self.free_handle_calls += 1
        if self.free_handle_calls == 1:
            raise RuntimeError("handle cleanup failed")
        super().free_handle(control)


class _PartialConnection:
    def __init__(
        self,
        acquisition_error: BaseException,
        cleanup_error: BaseException | None = None,
    ) -> None:
        self.acquisition_error = acquisition_error
        self.cleanup_error = cleanup_error
        self.close_calls = 0

    def connect(self, options) -> None:
        raise self.acquisition_error

    def runtime_client(self):
        raise AssertionError("failed connection must not publish a runtime client")

    def close(self) -> None:
        self.close_calls += 1
        if self.cleanup_error is not None:
            raise self.cleanup_error


class _SlowUnaryTransport:
    def __init__(
        self,
        *,
        delay: float = 0.0,
        release: threading.Event | None = None,
    ) -> None:
        self.delay = delay
        self.release = release
        self.closed = False
        self.started = threading.Event()
        self.invocations: list[object] = []
        self.signed_signers: list[object] = []

    def invoke(self, invocation):
        self.started.set()
        if self.release is not None:
            self.release.wait()
        if self.delay:
            time.sleep(self.delay)
        self.invocations.append(invocation)
        return {"ok": True}

    def invoke_signed(self, invocation, *, signer=None, options=None):
        self.started.set()
        if self.release is not None:
            self.release.wait()
        if self.delay:
            time.sleep(self.delay)
        self.invocations.append(invocation)
        self.signed_signers.append(signer)
        return {"ok": True}

    def close(self) -> None:
        self.closed = True


class _FailOnceUnaryTransport(_SlowUnaryTransport):
    def invoke(self, invocation):
        self.started.set()
        self.invocations.append(invocation)
        if len(self.invocations) == 1:
            raise RuntimeError("operation failed")
        return {"ok": True}


class _CloseFailsOnceUnaryTransport(_SlowUnaryTransport):
    def __init__(self, **kwargs) -> None:
        super().__init__(**kwargs)
        self.close_calls = 0

    def close(self) -> None:
        self.close_calls += 1
        if self.close_calls == 1:
            raise RuntimeError("close failed")
        super().close()


class _OperationAndCloseFailTransport(_CloseFailsOnceUnaryTransport):
    def invoke(self, invocation):
        self.started.set()
        if self.release is not None:
            self.release.wait()
        self.invocations.append(invocation)
        raise RuntimeError("operation failed")


class _BlockingCloseUnaryTransport(_SlowUnaryTransport):
    def __init__(
        self,
        *,
        release: threading.Event,
        close_release: threading.Event,
        close_error: BaseException | None = None,
    ) -> None:
        super().__init__(release=release)
        self.close_release = close_release
        self.close_error = close_error
        self.close_started = threading.Event()
        self.close_calls = 0

    def close(self) -> None:
        self.close_calls += 1
        self.close_started.set()
        self.close_release.wait()
        if self.close_error is not None:
            raise self.close_error
        super().close()


class _ObservedFlightLock:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._attempts_lock = threading.Lock()
        self._attempts = 0
        self.second_attempt = threading.Event()

    def acquire(
        self,
        blocking: bool = True,
        timeout: float = -1,
    ) -> bool:
        with self._attempts_lock:
            self._attempts += 1
            if self._attempts == 2:
                self.second_attempt.set()
        if not blocking:
            return self._lock.acquire(blocking=False)
        if timeout < 0:
            return self._lock.acquire()
        return self._lock.acquire(timeout=timeout)

    def release(self) -> None:
        self._lock.release()


class _MemoryBidiChannel:
    def __init__(
        self,
        *,
        frames: list[dict[str, object]] | None = None,
        close_error: str = "",
        timeout: bool = False,
    ) -> None:
        self.frames = list(frames or [])
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
    if not predicate():
        raise AssertionError("condition was not satisfied before deadline")


def _wait_for_thread(thread: threading.Thread, *, timeout: float) -> bool:
    thread.join(timeout=timeout)
    return not thread.is_alive()


def _sdk_unary_worker_count() -> int:
    return sum(thread.name == "easynet-sdk-unary" for thread in threading.enumerate())


def _sdk_unary_cleanup_worker_count() -> int:
    return sum(
        thread.name == "easynet-sdk-unary-cleanup" for thread in threading.enumerate()
    )


if __name__ == "__main__":
    unittest.main()
