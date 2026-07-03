import json
import unittest

from easynet_sdk import (
    ErrorCode,
    InvocationBuilder,
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
        self.handle_json = (
            b'{"handle_id":7,"state":"Submitted","terminal":false,'
            b'"events":[{"sequence":1,"kind":"submitted",'
            b'"state":"Submitted","terminal":false}],"result":null}'
        )

    def prepare(self, draft_json: bytes, options_json: bytes) -> bytes:
        if self.prepare_error is not None:
            raise self.prepare_error
        self.seen_draft = json.loads(draft_json.decode("utf-8"))
        self.seen_options = json.loads(options_json.decode("utf-8"))
        return PREPARED_FIXTURE

    def submit_signed(self, signed_json: bytes) -> bytes:
        self.seen_signed = json.loads(signed_json.decode("utf-8"))
        return self.handle_json


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


if __name__ == "__main__":
    unittest.main()
