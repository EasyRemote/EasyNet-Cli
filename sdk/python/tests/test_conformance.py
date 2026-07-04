import json
import pathlib
import unittest

from easynet_sdk import (
    ErrorCode,
    HOST_STREAM_FRAME_SCHEMA,
    HOST_STREAM_HASH_ALGORITHM,
    HostBindingClient,
    HostStreamBindingRequest,
    HostStreamEnvelope,
    HostStreamEnvelopeRequest,
    HostStreamHashState,
    HostStreamTerminalSummary,
    InvocationDraft,
    PreparedInvocation,
    RuntimeHealth,
    RetryHint,
    SDKError,
)


ROOT = pathlib.Path(__file__).resolve().parents[3]
SHARED_CONFORMANCE_FIXTURE_ROOT = "sdk/conformance/fixtures"
FIXTURES = ROOT / SHARED_CONFORMANCE_FIXTURE_ROOT


def shared_fixture(name: str) -> bytes:
    return (FIXTURES / name).read_bytes()


class SharedHostBindingTransport:
    def __init__(self) -> None:
        self.binding_json = shared_fixture("host-stream-binding.v4.json")
        self.request_json = shared_fixture("host-stream-request.v4.json")
        self.item_json = shared_fixture("host-stream-frame.v4.json")
        self.terminal_json = self._terminal_frame_json()
        self.hash_json = shared_fixture("host-stream-hash-state.v4.json")

    def _terminal_frame_json(self) -> bytes:
        summary = json.loads(shared_fixture("host-stream-terminal.v4.json"))
        return json.dumps(
            {
                "frame_type": "terminal",
                "seq": summary["frames"],
                "value": None,
                "error": None,
                "terminal": summary,
                "output_hash": summary["output_hash"],
            },
            separators=(",", ":"),
        ).encode("utf-8")

    def build_host_stream_binding(self, request_json: bytes) -> bytes:
        return self.binding_json

    def decode_request(self, envelope_json: bytes) -> bytes:
        return self.request_json

    def encode_item(self, request_json: bytes) -> bytes:
        return self.item_json

    def encode_error(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared conformance fixture test",
        )

    def encode_terminal(self, request_json: bytes) -> bytes:
        return self.terminal_json

    def fold_output_hash(self, request_json: bytes) -> bytes:
        return self.hash_json

    def close(self) -> None:
        return None


class SharedConformanceFixtureTests(unittest.TestCase):
    def test_python_facade_consumes_shared_runtime_core_fixtures(self) -> None:
        draft = InvocationDraft.from_json(shared_fixture("invocation.complete.v4.json"))
        self.assertEqual(draft.caller_ura, "easynet:///r/example/agent/alice.sdk")
        self.assertIn("args", json.loads(draft.to_json()))

        prepared = PreparedInvocation.from_json(
            shared_fixture("prepared.signing-material.v4.json")
        )
        self.assertFalse(prepared.submit_ready())
        self.assertEqual(prepared.signing_material.algorithm, "ed25519")

        runtime_error = SDKError.from_json(shared_fixture("runtime.error.v4.json"))
        assert runtime_error is not None
        self.assertEqual(runtime_error.code, ErrorCode.INVALID_ARGUMENT)
        self.assertEqual(runtime_error.retry, RetryHint.NEVER)

        health = RuntimeHealth.from_json(shared_fixture("health.ready.v4.json"))
        self.assertTrue(health.api_alive())
        self.assertTrue(health.ready())

    def test_python_host_binding_consumes_shared_conformance_fixtures(self) -> None:
        client = HostBindingClient(SharedHostBindingTransport())

        binding = client.build_host_stream_binding(
            HostStreamBindingRequest(
                binding_id="binding-weather-1",
                descriptor_ref="easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
                endpoint="/tmp/easynet-weather.sock",
                frame_schema=HOST_STREAM_FRAME_SCHEMA,
                cleanup={"mode": "unlink_socket"},
            )
        )
        self.assertEqual(binding.binding_id, "binding-weather-1")
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
        self.assertEqual(request.metadata["source"], "fixture")

        item = client.encode_item(0, {"token": "hello"})
        self.assertEqual(item.frame_type, "item")
        self.assertEqual(item.seq, 0)

        terminal = client.encode_terminal(
            HostStreamTerminalSummary(
                output_hash="sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15",
                frames=1,
            )
        )
        assert terminal.terminal is not None
        self.assertEqual(terminal.output_hash, terminal.terminal.output_hash)

        folded = client.fold_output_hash(
            HostStreamHashState(
                algorithm=HOST_STREAM_HASH_ALGORITHM,
                output_hash="sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                frames=0,
                last_seq=None,
            ),
            0,
            {"token": "hello"},
        )
        self.assertEqual(folded.last_seq, 0)
        self.assertEqual(folded.canonical_json, '{"token":"hello"}')


if __name__ == "__main__":
    unittest.main()
