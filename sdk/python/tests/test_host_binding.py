import json
import unittest

from easynet_sdk import ErrorCode, RetryHint, SDKError
from easynet_sdk.host_binding import (
    HOST_STREAM_FRAME_SCHEMA,
    HOST_STREAM_HASH_ALGORITHM,
    HostBindingClient,
    HostStreamBindingRequest,
    HostStreamEnvelope,
    HostStreamEnvelopeRequest,
    HostStreamHashState,
    HostStreamTerminalSummary,
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

    def _remember(self, request_json: bytes) -> None:
        self.seen_request = json.loads(request_json.decode("utf-8"))

    def build_host_stream_binding(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.binding_json

    def decode_request(self, envelope_json: bytes) -> bytes:
        self._remember(envelope_json)
        return self.request_json

    def encode_item(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.item_json

    def encode_error(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.error_json

    def encode_terminal(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.terminal_json

    def fold_output_hash(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.hash_json


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
        state = HostStreamHashState(
            algorithm=HOST_STREAM_HASH_ALGORITHM,
            output_hash="sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            frames=0,
            last_seq=None,
        )

        folded = client.fold_output_hash(state, 0, {"token": "hello"})
        self.assertEqual(folded.last_seq, 0)
        self.assertEqual(folded.canonical_json, '{"token":"hello"}')

        with self.assertRaises(SDKError):
            client.fold_output_hash(state, 2, {"token": "skip"})


if __name__ == "__main__":
    unittest.main()
