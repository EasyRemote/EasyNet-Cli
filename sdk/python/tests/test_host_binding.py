import json
import unittest

from easynet_sdk import ErrorCode, RetryHint, SDKError, is_code
from easynet_sdk.host_binding import (
    HOST_STREAM_FRAME_SCHEMA,
    HOST_STREAM_HASH_ALGORITHM,
    HOST_STREAM_EMPTY_OUTPUT_HASH,
    HostBindingClient,
    HostStreamBindingRequest,
    HostStreamCleanup,
    HostStreamEnvelope,
    HostStreamEnvelopeRequest,
    HostStreamHashState,
    HostStreamLifecycle,
    HostStreamReadiness,
    HostStreamSessionState,
    HostStreamTerminalSummary,
    LocalHostBindingTransport,
)


BINDING_JSON = b"""{
  "binding_id": "binding-weather-1",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
  "endpoint": "/tmp/easynet-weather.sock",
  "frame_schema": "host-stream-frame.schema.json",
  "cleanup": {"mode": "unlink_socket"},
  "timeout_ms": 30000,
  "readiness": {"state": "declared", "checked": false, "endpoint_ready": null},
  "lifecycle": {
    "endpoint_owner": "product_host",
    "process_owner": "product_host",
    "frame_contract_owner": "daemon_sdk"
  },
  "metadata": {
    "profile": "host_binding",
    "source": "fixture",
    "frame_schema": "host-stream-frame.schema.json",
    "hash_algorithm": "sha256(prev_hash || seq_be || canonical_json(value))"
  }
}"""

REQUEST_JSON = b"""{
  "function": "weather.stream",
  "args": {"city": "Singapore"},
  "call_id": "call-weather-1",
  "caller": "easynet:///r/example/user/alice",
  "metadata": {"wire": "host_stream_request_v1", "source": "fixture"}
}"""

ITEM_FRAME_JSON = b"""{
  "frame_type": "item",
  "seq": 0,
  "value": {"token": "hello"},
  "error": null,
  "terminal": null,
  "output_hash": null
}"""

ERROR_FRAME_JSON = b"""{
  "frame_type": "error",
  "seq": null,
  "value": null,
  "error": {
    "code": "InvalidArgument",
    "stage": "host",
    "message": "bad input",
    "retry": "never",
    "details": {}
  },
  "terminal": null,
  "output_hash": null
}"""

TERMINAL_FRAME_JSON = b"""{
  "frame_type": "terminal",
  "seq": 1,
  "value": null,
  "error": null,
  "terminal": {
    "output_hash": "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15",
    "frames": 1,
    "metadata": {"canonical_json": "{\\"token\\":\\"hello\\"}"}
  },
  "output_hash": "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15"
}"""

HASH_STATE_JSON = b"""{
  "algorithm": "sha256(prev_hash || seq_be || canonical_json(value))",
  "output_hash": "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15",
  "frames": 1,
  "last_seq": 0,
  "canonical_json": "{\\"token\\":\\"hello\\"}"
}"""


class MemoryHostBindingTransport:
    def __init__(self) -> None:
        self.binding_json = BINDING_JSON
        self.request_json = REQUEST_JSON
        self.item_json = ITEM_FRAME_JSON
        self.error_json = ERROR_FRAME_JSON
        self.terminal_json = TERMINAL_FRAME_JSON
        self.hash_json = HASH_STATE_JSON
        self.seen_request: dict[str, object] | None = None
        self.calls: list[str] = []
        self.close_calls = 0

    def _remember(self, request_json: bytes) -> None:
        self.seen_request = json.loads(request_json.decode("utf-8"))

    def build_host_stream_binding(self, request_json: bytes) -> bytes:
        self.calls.append("build_host_stream_binding")
        self._remember(request_json)
        return self.binding_json

    def decode_request(self, envelope_json: bytes) -> bytes:
        self.calls.append("decode_request")
        self._remember(envelope_json)
        return self.request_json

    def encode_item(self, request_json: bytes) -> bytes:
        self.calls.append("encode_item")
        self._remember(request_json)
        return self.item_json

    def encode_error(self, request_json: bytes) -> bytes:
        self.calls.append("encode_error")
        self._remember(request_json)
        return self.error_json

    def encode_terminal(self, request_json: bytes) -> bytes:
        self.calls.append("encode_terminal")
        self._remember(request_json)
        return self.terminal_json

    def fold_output_hash(self, request_json: bytes) -> bytes:
        self.calls.append("fold_output_hash")
        self._remember(request_json)
        return self.hash_json

    def close(self) -> None:
        self.close_calls += 1


class HostBindingTests(unittest.TestCase):
    def test_build_binding_and_decode_request(self) -> None:
        transport = MemoryHostBindingTransport()
        client = HostBindingClient(transport)

        binding = client.build_host_stream_binding(
            HostStreamBindingRequest(
                binding_id="binding-weather-1",
                descriptor_ref="easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
                endpoint="/tmp/easynet-weather.sock",
                cleanup={"mode": "unlink_socket"},
                metadata={"owner": "easyremote"},
            )
        )
        self.assertEqual(binding.frame_schema, HOST_STREAM_FRAME_SCHEMA)
        self.assertIsInstance(binding.cleanup, HostStreamCleanup)
        self.assertIsInstance(binding.readiness, HostStreamReadiness)
        self.assertIsInstance(binding.lifecycle, HostStreamLifecycle)
        self.assertEqual(binding.cleanup.mode, "unlink_socket")
        self.assertEqual(binding.readiness.state, "declared")
        self.assertEqual(binding.lifecycle["frame_contract_owner"], "daemon_sdk")

        request = client.decode_request(
            HostStreamEnvelope(
                HostStreamEnvelopeRequest(
                    fn="weather.stream",
                    args={"city": "Singapore"},
                    call_id="call-weather-1",
                    caller="easynet:///r/example/user/alice",
                )
            )
        )
        self.assertEqual(request.function, "weather.stream")
        self.assertEqual(request.metadata["wire"], "host_stream_request_v1")
        self.assertIsNone(request.parent_receipt)

    def test_decode_request_preserves_parent_receipt_anchor(self) -> None:
        client = HostBindingClient(LocalHostBindingTransport())
        parent_receipt = {
            "receipt_ura": "easynet:///r/example/receipt/parent-1",
            "invocation_id": "inv-parent-1",
            "self_hash_hex": "aa" * 32,
        }

        request = client.decode_request(
            HostStreamEnvelope(
                HostStreamEnvelopeRequest(
                    fn="weather.child",
                    args={"city": "Singapore"},
                    call_id="call-weather-2",
                    caller="easynet:///r/example/user/alice",
                    parent_receipt=parent_receipt,
                )
            )
        )

        self.assertEqual(request.parent_receipt, parent_receipt)

    def test_rejects_relative_endpoint_and_schema_drift(self) -> None:
        client = HostBindingClient(MemoryHostBindingTransport())

        with self.assertRaises(SDKError):
            client.build_host_stream_binding(
                HostStreamBindingRequest(
                    binding_id="binding-weather-1",
                    descriptor_ref="easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
                    endpoint="tmp/easynet-weather.sock",
                )
            )

        with self.assertRaises(SDKError):
            client.build_host_stream_binding(
                HostStreamBindingRequest(
                    binding_id="binding-weather-1",
                    descriptor_ref="easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
                    endpoint="/tmp/easynet-weather.sock",
                    frame_schema="other.schema.json",
                )
            )

    def test_local_transport_rejects_descriptor_and_endpoint_drift(self) -> None:
        client = HostBindingClient(LocalHostBindingTransport())

        with self.assertRaises(SDKError):
            client.build_host_stream_binding(
                HostStreamBindingRequest(
                    binding_id="binding-weather-1",
                    descriptor_ref="easynet:///r/example/ability/device.dev-a.weather.stream",
                    endpoint="/tmp/easynet-weather.sock",
                )
            )

        with self.assertRaises(SDKError):
            client.build_host_stream_binding(
                HostStreamBindingRequest(
                    binding_id="binding-weather-1",
                    descriptor_ref="easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
                    endpoint="/tmp/../easynet-weather.sock",
                )
            )

    def test_local_transport_uses_injected_descriptor_canonicalizer(self) -> None:
        def canonicalize(value: str) -> str:
            self.assertEqual(value, "weather.stream@1")
            return "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0"

        client = HostBindingClient(LocalHostBindingTransport(canonicalize))
        binding = client.build_host_stream_binding(
            HostStreamBindingRequest(
                binding_id="binding-weather-1",
                descriptor_ref="weather.stream@1",
                endpoint="/tmp/easynet-weather.sock",
            )
        )

        self.assertEqual(
            binding.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
        )

    def test_local_transport_rejects_incomplete_envelope(self) -> None:
        client = HostBindingClient(LocalHostBindingTransport())

        with self.assertRaises(SDKError):
            client.decode_request(
                HostStreamEnvelope.from_json(
                    b'{"request":{"fn":"weather.stream",'
                    b'"args":{"city":"Singapore"},"call_id":"call-weather-1"}}'
                )
            )

    def test_binding_request_accepts_typed_lifecycle_dtos(self) -> None:
        transport = MemoryHostBindingTransport()
        client = HostBindingClient(transport)

        client.build_host_stream_binding(
            HostStreamBindingRequest(
                binding_id="binding-weather-1",
                descriptor_ref="easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
                endpoint="/tmp/easynet-weather.sock",
                cleanup=HostStreamCleanup(mode="unlink_socket"),
                readiness=HostStreamReadiness(
                    state="declared",
                    checked=False,
                    endpoint_ready=None,
                    metadata={"probe": "socket_exists"},
                ),
                timeout_ms=30000,
            )
        )

        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["cleanup"], {"mode": "unlink_socket"})
        self.assertEqual(
            transport.seen_request["readiness"],
            {
                "checked": False,
                "endpoint_ready": None,
                "probe": "socket_exists",
                "state": "declared",
            },
        )

    def test_encodes_frame_variants(self) -> None:
        client = HostBindingClient(MemoryHostBindingTransport())

        item = client.encode_item(0, {"token": "hello"})
        self.assertEqual(item.frame_type, "item")
        self.assertEqual(item.seq, 0)
        self.assertEqual(item.value["token"], "hello")

        error_frame = client.encode_error(
            SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="host",
                retry=RetryHint.NEVER,
                message="bad input",
            )
        )
        self.assertEqual(error_frame.frame_type, "error")
        assert error_frame.error is not None
        self.assertEqual(error_frame.error.code, ErrorCode.INVALID_ARGUMENT)

        terminal = client.encode_terminal(
            HostStreamTerminalSummary(
                output_hash="sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15",
                frames=1,
            )
        )
        self.assertEqual(terminal.frame_type, "terminal")
        self.assertEqual(terminal.output_hash, terminal.terminal.output_hash)

    def test_fold_output_hash_rejects_sequence_gap(self) -> None:
        client = HostBindingClient(MemoryHostBindingTransport())
        state = HostStreamHashState.initial()

        folded = client.fold_output_hash(state, 0, {"token": "hello"})
        self.assertEqual(folded.last_seq, 0)
        self.assertEqual(folded.canonical_json, '{"token":"hello"}')

        with self.assertRaises(SDKError):
            client.fold_output_hash(state, 2, {"token": "skip"})

    def test_hash_state_rejects_corrupted_frame_cursor(self) -> None:
        corrupted_zero = json.loads(HASH_STATE_JSON.decode("utf-8"))
        corrupted_zero["frames"] = 0
        corrupted_zero["last_seq"] = 0

        with self.assertRaises(SDKError) as caught:
            HostStreamHashState.from_json(json.dumps(corrupted_zero))
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn("cannot have last_seq", caught.exception.message)

        corrupted_gap = json.loads(HASH_STATE_JSON.decode("utf-8"))
        corrupted_gap["frames"] = 3
        corrupted_gap["last_seq"] = 0

        with self.assertRaises(SDKError) as caught:
            HostStreamHashState.from_json(json.dumps(corrupted_gap))
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn("last_seq must match frames", caught.exception.message)

    def test_fold_output_hash_rejects_corrupted_local_state(self) -> None:
        transport = MemoryHostBindingTransport()
        client = HostBindingClient(transport)
        state = HostStreamHashState(
            algorithm=HOST_STREAM_HASH_ALGORITHM,
            output_hash=HOST_STREAM_EMPTY_OUTPUT_HASH,
            frames=2,
            last_seq=0,
        )

        with self.assertRaises(SDKError) as caught:
            client.fold_output_hash(state, 2, {"token": "late"})
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn("last_seq must match frames", caught.exception.message)
        self.assertEqual(transport.calls, [])

    def test_local_transport_rejects_non_canonical_previous_hash(self) -> None:
        client = HostBindingClient(LocalHostBindingTransport())
        with self.assertRaises(SDKError):
            client.fold_output_hash(
                HostStreamHashState(
                    algorithm=HOST_STREAM_HASH_ALGORITHM,
                    output_hash="sha256:ABC",
                    frames=0,
                    last_seq=None,
                ),
                0,
                {"token": "hello"},
            )

    def test_frame_writer_sequences_items_and_terminal_via_client(self) -> None:
        transport = MemoryHostBindingTransport()
        client = HostBindingClient(transport)
        writer = client.open_frame_writer()

        self.assertEqual(writer.output_hash, HOST_STREAM_EMPTY_OUTPUT_HASH)
        item = writer.write_item({"token": "hello"})
        self.assertEqual(item.frame_type, "item")
        self.assertEqual(item.seq, 0)
        self.assertEqual(writer.frames, 1)
        self.assertEqual(
            writer.output_hash,
            json.loads(HASH_STATE_JSON.decode("utf-8"))["output_hash"],
        )
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["seq"], 0)
        self.assertEqual(transport.calls, ["fold_output_hash", "encode_item"])

        terminal = writer.finish(metadata={"canonical_json": writer.state.canonical_json})
        self.assertEqual(terminal.frame_type, "terminal")
        self.assertTrue(writer.terminal)
        self.assertEqual(
            transport.calls,
            ["fold_output_hash", "encode_item", "encode_terminal"],
        )

        with self.assertRaises(SDKError):
            writer.write_item({"token": "late"})

    def test_local_transport_encodes_daemon_host_stream_wire(self) -> None:
        client = HostBindingClient(LocalHostBindingTransport())
        writer = client.open_frame_writer()

        item = writer.write_item("a:hi")
        second = writer.write_item("b:hi")
        third = writer.write_item("c:hi")
        terminal = writer.finish()

        self.assertEqual(item.to_host_wire_dict(), {"stream_item": "a:hi", "seq": 0})
        self.assertEqual(second.to_host_wire_dict(), {"stream_item": "b:hi", "seq": 1})
        self.assertEqual(third.to_host_wire_dict(), {"stream_item": "c:hi", "seq": 2})
        self.assertEqual(
            terminal.to_host_wire_dict(),
            {
                "terminal": {
                    "output_hash": (
                        "sha256:653e1bed022d2aa75fba7d09f92bb1d1db86c3caffb89cf54e6f7556ff3e3183"
                    ),
                    "frames": 3,
                }
            },
        )

    def test_local_transport_preserves_host_wire_error_taxonomy(self) -> None:
        client = HostBindingClient(LocalHostBindingTransport())
        frame = client.encode_error(
            SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="host",
                retry=RetryHint.NEVER,
                message="bad input",
                details={"kind": "INVALID_ARGUMENT", "reason": "bad_request"},
            )
        )

        self.assertEqual(
            frame.to_host_wire_dict(),
            {
                "error": {
                    "kind": "INVALID_ARGUMENT",
                    "reason": "bad_request",
                    "message": "bad input",
                }
            },
        )

    def test_frame_writer_fail_is_terminal_and_does_not_close_client(self) -> None:
        transport = MemoryHostBindingTransport()
        client = HostBindingClient(transport)
        writer = client.open_frame_writer()

        frame = writer.fail(ValueError("bad input"))
        self.assertEqual(frame.frame_type, "error")
        self.assertTrue(writer.terminal)
        self.assertEqual(transport.close_calls, 0)

        with self.assertRaises(SDKError):
            writer.finish()

        writer.close()
        self.assertEqual(transport.close_calls, 0)

    def test_open_session_decodes_request_and_owns_frame_state(self) -> None:
        transport = MemoryHostBindingTransport()
        client = HostBindingClient(transport)

        session = client.open_session(_weather_envelope())

        self.assertEqual(session.request.function, "weather.stream")
        self.assertEqual(session.state, HostStreamSessionState.OPEN)
        item = session.emit({"token": "hello"})
        terminal = session.finish(
            metadata={"canonical_json": session.writer.state.canonical_json}
        )

        self.assertEqual(item.frame_type, "item")
        self.assertEqual(terminal.frame_type, "terminal")
        self.assertEqual(session.state, HostStreamSessionState.TERMINAL)
        self.assertIs(session.terminal_frame, terminal)
        self.assertEqual(
            transport.calls,
            [
                "decode_request",
                "fold_output_hash",
                "encode_item",
                "encode_terminal",
            ],
        )
        with self.assertRaises(SDKError):
            session.emit({"token": "late"})

    def test_host_stream_session_fail_is_single_terminal(self) -> None:
        transport = MemoryHostBindingTransport()
        client = HostBindingClient(transport)
        session = client.open_session(_weather_envelope())

        frame = session.fail(ValueError("bad input"))

        self.assertEqual(frame.frame_type, "error")
        self.assertEqual(session.state, HostStreamSessionState.TERMINAL)
        self.assertIs(session.terminal_frame, frame)
        with self.assertRaises(SDKError):
            session.finish()

    def test_host_stream_session_close_is_idempotent_without_terminal_frame(self) -> None:
        transport = MemoryHostBindingTransport()
        client = HostBindingClient(transport)
        session = client.open_session(_weather_envelope())

        session.close()
        session.close()

        self.assertEqual(session.state, HostStreamSessionState.CLOSED)
        self.assertIsNone(session.terminal_frame)
        with self.assertRaises(SDKError):
            session.emit({"token": "late"})

    def test_close_delegates_once_and_fails_closed(self) -> None:
        transport = MemoryHostBindingTransport()
        client = HostBindingClient(transport)

        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            client.encode_item(0, {"token": "hello"})
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(transport.seen_request)


def _weather_envelope() -> HostStreamEnvelope:
    return HostStreamEnvelope(
        HostStreamEnvelopeRequest(
            fn="weather.stream",
            args={"city": "Singapore"},
            call_id="call-weather-1",
            caller="easynet:///r/example/user/alice",
        )
    )


if __name__ == "__main__":
    unittest.main()
