import json
import unittest

from easynet_sdk import (
    ErrorCode,
    InvocationBuilder,
    InvocationResult,
    InvocationSignature,
    PrepareOptions,
    PreparedInvocation,
    RuntimeClient,
    SDKError,
    is_code,
)

from test_signing import PREPARED_FIXTURE


class MemoryRuntimeTransport:
    def __init__(self) -> None:
        self.seen_draft: dict[str, object] | None = None
        self.seen_options: dict[str, object] | None = None
        self.seen_signed: dict[str, object] | None = None
        self.prepare_error: BaseException | None = None
        self.seen_await_id = 0
        self.seen_cancel_reason = ""
        self.handle_json = (
            b'{"handle_id":7,"state":"Submitted","terminal":false,'
            b'"events":[{"sequence":1,"kind":"submitted",'
            b'"state":"Submitted","terminal":false}],"result":null}'
        )

    def invoke(self, draft_json: bytes) -> bytes:
        self.seen_draft = json.loads(draft_json.decode("utf-8"))
        return json.dumps(
            {
                "ok": True,
                "tuple": self.seen_draft,
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_base64": "eyJyZWFkeSI6dHJ1ZX0=",
                "output_json": {"ready": True},
                "selected_node_id": "node-a",
                "scheduling_reason": "direct",
                "elapsed_ms": 12,
                "receipt": {"receipt_id": "receipt-1"},
                "error": None,
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")

    def prepare(self, draft_json: bytes, options_json: bytes) -> bytes:
        if self.prepare_error is not None:
            raise self.prepare_error
        self.seen_draft = json.loads(draft_json.decode("utf-8"))
        self.seen_options = json.loads(options_json.decode("utf-8"))
        return PREPARED_FIXTURE

    def submit_signed(self, signed_json: bytes) -> bytes:
        self.seen_signed = json.loads(signed_json.decode("utf-8"))
        return self.handle_json

    def await_handle(self, handle_id: int) -> bytes:
        self.seen_await_id = handle_id
        draft = self.seen_draft or complete_draft().to_json_dict()
        return json.dumps(
            {
                "ok": True,
                "tuple": draft,
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_base64": "e30=",
                "output_json": {},
                "elapsed_ms": 8,
                "receipt": None,
                "error": None,
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")

    def cancel_handle(self, handle_id: int, reason: str) -> bytes:
        self.seen_cancel_reason = reason
        return (
            b'{"handle_id":7,"cancelled":true,'
            b'"state":"Cancelled","terminal":true}'
        )

    def handle_events(self, handle_id: int) -> bytes:
        return (
            b'{"handle_id":7,"state":"Cancelled","terminal":true,'
            b'"events":[{"sequence":1,"kind":"submitted",'
            b'"state":"Submitted","terminal":false},{"sequence":2,'
            b'"kind":"cancelled","state":"Cancelled","terminal":true,'
            b'"reason":"client stop"}],"result":null}'
        )


def complete_draft():
    return (
        InvocationBuilder()
        .with_caller_ura("easynet:///r/example/agent/alice.sdk")
        .with_callee_ura("easynet:///r/example/device/dev-a")
        .with_descriptor_ref(
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
        )
        .with_subject_ura("easynet:///r/example/device/dev-a")
        .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
        .with_causal_context({"form": "none"})
        .with_json_args({})
        .with_content_type("application/json")
        .build()
    )


def signed_fixture():
    prepared = PreparedInvocation.from_json(PREPARED_FIXTURE)
    return prepared.sign_with_caller_signature(
        InvocationSignature(
            algorithm="ed25519",
            signature_base64="c2lnbmF0dXJl",
            key_id_hint="caller-key",
        )
    )


class RuntimeTests(unittest.TestCase):
    def test_invoke_returns_typed_result(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        result = client.invoke(complete_draft())

        self.assertTrue(result.ok)
        self.assertEqual(result.terminal_state, "Completed")
        self.assertEqual(
            result.tuple.caller_ura,
            "easynet:///r/example/agent/alice.sdk",
        )
        self.assertEqual(result.output_json, {"ready": True})
        assert transport.seen_draft is not None
        self.assertEqual(
            transport.seen_draft["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )

    def test_prepare_delegates_to_transport(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        prepared, material = client.prepare(
            complete_draft(), PrepareOptions(expires_in_ms=60000)
        )

        self.assertFalse(prepared.submit_ready())
        self.assertTrue(material.canonical_bytes_base64)
        assert transport.seen_draft is not None
        self.assertEqual(
            transport.seen_draft["caller_ura"],
            "easynet:///r/example/agent/alice.sdk",
        )
        self.assertEqual(transport.seen_options, {"expires_in_ms": 60000})

    def test_submit_signed_preserves_signature(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)

        handle = client.submit_signed(signed_fixture())

        self.assertEqual(handle.handle_id, 7)
        self.assertEqual(handle.state, "Submitted")
        self.assertFalse(handle.terminal)
        self.assertEqual(handle.events[0].sequence, 1)
        assert transport.seen_signed is not None
        self.assertEqual(
            transport.seen_signed["signature"]["signature_base64"],
            "c2lnbmF0dXJl",
        )

    def test_handle_observation_delegates_to_transport(self) -> None:
        transport = MemoryRuntimeTransport()
        client = RuntimeClient(transport)
        handle = client.submit_signed(signed_fixture())

        result = client.await_result(handle)
        cancelled = client.cancel(handle, "client stop")
        events = client.events(handle)

        self.assertTrue(result.ok)
        self.assertEqual(transport.seen_await_id, 7)
        self.assertTrue(cancelled.cancelled)
        self.assertTrue(cancelled.terminal)
        self.assertEqual(transport.seen_cancel_reason, "client stop")
        self.assertTrue(events.terminal)
        self.assertEqual(len(events.events), 2)
        self.assertEqual(events.events[1].reason, "client stop")

    def test_prepare_wraps_transport_failure(self) -> None:
        transport = MemoryRuntimeTransport()
        transport.prepare_error = RuntimeError("daemon unavailable")
        client = RuntimeClient(transport)

        with self.assertRaises(SDKError) as caught:
            client.prepare(complete_draft())

        self.assertTrue(is_code(caught.exception, ErrorCode.TRANSPORT))
        self.assertIsInstance(caught.exception.cause, RuntimeError)

    def test_submit_rejects_malformed_handle(self) -> None:
        transport = MemoryRuntimeTransport()
        transport.handle_json = b'{"state":"Submitted"}'
        client = RuntimeClient(transport)

        with self.assertRaises(SDKError) as caught:
            client.submit_signed(signed_fixture())

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_result_rejects_inconsistent_failure(self) -> None:
        result = {
            "ok": False,
            "tuple": complete_draft().to_json_dict(),
            "terminal_state": "Failed",
            "output_content_type": "application/json",
            "output_base64": "",
            "output_json": None,
            "elapsed_ms": 3,
            "receipt": None,
            "error": None,
        }

        with self.assertRaises(SDKError) as caught:
            InvocationResult.from_json(json.dumps(result))

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))


if __name__ == "__main__":
    unittest.main()
