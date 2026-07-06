import json
import unittest

from easynet_sdk import (
    AbilityCallRequest,
    AbilityInvocationClient,
    AbilityTargetRequest,
    BidiStreamDescriptor,
    ErrorCode,
    InvocationResult,
    InvocationSignature,
    PrepareOptions,
    ReceiptClient,
    RuntimeClient,
    SDKError,
    Signer,
    is_code,
)
from easynet_sdk.identity import AddressingClient

from test_identity import MemoryIdentityTransport
from test_runtime import MemoryRuntimeTransport
from test_signing import signer_handle


ABILITY_URA = "easynet:///r/example/ability/device.dev-a.observe.health"
DESCRIPTOR_REF = f"{ABILITY_URA}@1.0.0"


class AbilityInvocationClientTests(unittest.TestCase):
    def test_build_invocation_from_ability_ura_delegates_descriptor_ref(self) -> None:
        identity = _identity_transport()
        runtime = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime),
            addressing=AddressingClient(identity),
        )

        draft = client.build_invocation(_request(ability_ura=ABILITY_URA))

        self.assertEqual(
            draft.descriptor_ref,
            DESCRIPTOR_REF,
        )
        self.assertEqual(draft.subject_ura, "easynet:///r/example/device/dev-a")
        self.assertEqual(draft.causal_context, {"form": "none"})
        self.assertEqual(
            identity.seen_requests,
            [
                {"ability_ura": ABILITY_URA, "descriptor_version": "1.0.0"},
            ],
        )

    def test_build_invocation_from_ability_ura_uses_descriptor_builder(self) -> None:
        identity = _identity_transport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(MemoryRuntimeTransport()),
            addressing=AddressingClient(identity),
        )

        draft = client.build_invocation(
            _request(ability_ura=ABILITY_URA)
        )

        self.assertEqual(
            draft.descriptor_ref,
            DESCRIPTOR_REF,
        )
        self.assertEqual(
            identity.seen_requests,
            [
                {"ability_ura": ABILITY_URA, "descriptor_version": "1.0.0"}
            ],
        )

    def test_build_invocation_from_descriptor_ref_canonicalizes_ref(self) -> None:
        identity = _identity_transport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(MemoryRuntimeTransport()),
            addressing=AddressingClient(identity),
        )

        draft = client.build_invocation(
            _request(
                descriptor_ref=(
                    DESCRIPTOR_REF
                )
            )
        )

        self.assertEqual(
            draft.descriptor_ref,
            DESCRIPTOR_REF,
        )
        self.assertEqual(
            identity.seen_requests,
            [
                {
                    "descriptor_ref": (
                        DESCRIPTOR_REF
                    )
                }
            ],
        )

    def test_invoke_stream_and_bidi_dispatch_built_draft(self) -> None:
        identity = _identity_transport()
        runtime_transport = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime_transport),
            addressing=AddressingClient(identity),
        )

        result = client.invoke(_request(ability_ura=ABILITY_URA))
        stream = client.stream(_request(ability_ura=ABILITY_URA))
        event = stream.next()
        stream.close()
        bidi = client.bidi(
            _request(ability_ura=ABILITY_URA),
            (BidiStreamDescriptor(stream_id=1, content_type="application/json"),),
        )
        bidi.close_send()
        bidi.cancel("done")
        bidi.close()

        self.assertTrue(result.ok)
        self.assertTrue(event.terminal)
        assert runtime_transport.seen_draft is not None
        self.assertEqual(
            runtime_transport.seen_draft["descriptor_ref"],
            DESCRIPTOR_REF,
        )
        self.assertEqual(
            runtime_transport.seen_streams,
            [{"content_type": "application/json", "stream_id": 1}],
        )

    def test_target_invocation_from_ability_ura_derives_tuple_facts(self) -> None:
        identity = _identity_transport()
        runtime = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime),
            addressing=AddressingClient(identity),
        )

        result = client.invoke_target(
            _target_request(
                ability_ura=ABILITY_URA
            )
        )

        self.assertTrue(result.ok)
        assert runtime.seen_draft is not None
        self.assertEqual(
            runtime.seen_draft["callee_ura"],
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(
            runtime.seen_draft["subject_ura"],
            ABILITY_URA,
        )
        self.assertEqual(
            runtime.seen_draft["descriptor_ref"],
            DESCRIPTOR_REF,
        )
        self.assertEqual(
            identity.seen_requests,
            [
                {"ability_ura": ABILITY_URA, "descriptor_version": "1.0.0"},
                {"ura": ABILITY_URA},
            ],
        )

    def test_target_invocation_from_descriptor_ref_uses_projection_once(self) -> None:
        identity = _identity_transport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(MemoryRuntimeTransport()),
            addressing=AddressingClient(identity),
        )

        draft = client.build_target_invocation(
            _target_request(
                descriptor_ref=(
                    DESCRIPTOR_REF
                )
            )
        )

        self.assertEqual(draft.callee_ura, "easynet:///r/example/device/dev-a")
        self.assertEqual(
            draft.subject_ura,
            "easynet:///r/example/ability/device.dev-a.observe.health",
        )
        self.assertEqual(
            identity.seen_requests,
            [
                {
                    "descriptor_ref": (
                        DESCRIPTOR_REF
                    )
                },
                {"ura": ABILITY_URA},
            ],
        )

    def test_prepare_delegates_built_invocation_without_signing_locally(self) -> None:
        identity = _identity_transport()
        runtime = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime),
            addressing=AddressingClient(identity),
        )

        prepared, material = client.prepare(
            _request(ability_ura=ABILITY_URA),
            PrepareOptions(
                expires_in_ms=60000,
                signer_id="signer-alice-key-1",
                policy_ref="daemon-key-inventory:sha256:test-policy",
                local_daemon_signing=True,
            ),
        )

        self.assertEqual(prepared.prepared_id, "prepared-example-1")
        self.assertTrue(material.canonical_bytes_base64)
        self.assertIsNone(runtime.seen_signed)
        self.assertEqual(
            runtime.seen_options,
            {
                "expires_in_ms": 60000,
                "signer_id": "signer-alice-key-1",
                "policy_ref": "daemon-key-inventory:sha256:test-policy",
                "local_daemon_signing": True,
            },
        )
        assert runtime.seen_draft is not None
        self.assertEqual(
            runtime.seen_draft["descriptor_ref"],
            DESCRIPTOR_REF,
        )
        self.assertEqual(
            identity.seen_requests,
            [
                {"ability_ura": ABILITY_URA, "descriptor_version": "1.0.0"},
            ],
        )

    def test_prepare_and_sign_target_keeps_submission_explicit(self) -> None:
        identity = _identity_transport()
        runtime = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime),
            addressing=AddressingClient(identity),
        )
        signer = Signer.from_signature(
            signer_handle(),
            InvocationSignature(
                algorithm="ed25519",
                signature_base64="c2lnbmF0dXJl",
            ),
        )

        signed, material = client.prepare_and_sign_target(
            _target_request(
                ability_ura=ABILITY_URA
            ),
            signer,
            PrepareOptions(expires_in_ms=60000),
        )

        self.assertTrue(signed.submit_ready())
        self.assertTrue(material.canonical_bytes_base64)
        self.assertIsNone(runtime.seen_signed)
        self.assertEqual(runtime.seen_options, {"expires_in_ms": 60000})
        assert runtime.seen_draft is not None
        self.assertEqual(
            runtime.seen_draft["callee_ura"],
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(
            runtime.seen_draft["subject_ura"],
            ABILITY_URA,
        )

        handle = client.submit_signed(signed)
        result = client.await_result(handle)
        cancelled = client.cancel(handle, "client stop")
        events = client.events(handle)
        client.close_handle(handle)

        self.assertEqual(handle.handle_id, 7)
        self.assertTrue(result.ok)
        self.assertTrue(cancelled.cancelled)
        self.assertTrue(events.terminal)
        self.assertEqual(runtime.seen_await_id, 7)
        self.assertEqual(runtime.seen_cancel_reason, "client stop")
        self.assertEqual(runtime.seen_free_id, 7)
        assert runtime.seen_signed is not None
        self.assertEqual(runtime.seen_signed["signer_id"], "signer-alice-key-1")
        self.assertEqual(
            identity.seen_requests,
            [
                {"ability_ura": ABILITY_URA, "descriptor_version": "1.0.0"},
                {"ura": ABILITY_URA},
            ],
        )

    def test_target_invocation_rejects_ambiguous_or_incomplete_selectors(self) -> None:
        runtime = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime),
            addressing=AddressingClient(_identity_transport()),
        )

        for request in (
            _target_request(),
            _target_request(
                ability_ura=(
                    " easynet:///r/example/ability/device.dev-a.observe.health"
                )
            ),
            _target_request(
                ability_ura=ABILITY_URA,
                descriptor_ref=DESCRIPTOR_REF,
            ),
        ):
            with self.subTest(request=request):
                with self.assertRaises(SDKError) as caught:
                    client.build_target_invocation(request)
                self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

        self.assertIsNone(runtime.seen_draft)

    def test_rejects_incomplete_tuple_and_selector_ambiguity(self) -> None:
        client = AbilityInvocationClient(
            runtime=RuntimeClient(MemoryRuntimeTransport()),
            addressing=AddressingClient(_identity_transport()),
        )

        with self.assertRaises(SDKError) as caught:
            client.build_invocation(_request())
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

        with self.assertRaises(SDKError):
            client.build_invocation(
                _request(
                    descriptor_ref=DESCRIPTOR_REF,
                    ability_ura=ABILITY_URA,
                )
            )

        with self.assertRaises(SDKError):
            client.build_invocation(
                AbilityCallRequest(
                    caller_ura="",
                    callee_ura="easynet:///r/example/device/dev-a",
                    subject_ura="easynet:///r/example/device/dev-a",
                    nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                    causal_context={"form": "none"},
                    ability_ura=ABILITY_URA,
                )
            )

    def test_rejects_surrounding_whitespace_before_dispatch(self) -> None:
        runtime = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime),
            addressing=AddressingClient(_identity_transport()),
        )

        with self.assertRaises(SDKError) as caught:
            client.invoke(_request(ability_ura=f" {ABILITY_URA}"))
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

        with self.assertRaises(SDKError) as caught:
            client.invoke(
                _request_with(
                    caller_ura=" easynet:///r/example/agent/alice.sdk",
                    ability_ura=ABILITY_URA,
                )
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(runtime.seen_draft)

    def test_child_context_anchors_child_invocation_to_parent_receipt(self) -> None:
        identity = _identity_transport()
        runtime = MemoryRuntimeTransport()
        receipt_transport = ChildReceiptTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime),
            addressing=AddressingClient(identity),
        )
        parent = _parent_result_with_receipt()

        child = client.child_context(
            parent,
            ReceiptClient(receipt_transport),
            caller_ura="easynet:///r/example/agent/child.sdk",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            metadata={"trace_id": "parent-1"},
        )
        request = child.call_request(
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            ability_ura=ABILITY_URA,
            args={"child": True},
            metadata={"attempt": 1},
        )

        result = child.invoke(
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            ability_ura=ABILITY_URA,
            args={"child": True},
            metadata={"attempt": 1},
        )

        self.assertTrue(result.ok)
        self.assertEqual(
            request.causal_context,
            {
                "form": "scalar",
                "receipt_ura": "easynet:///r/example/receipt/parent-1",
                "receipt_hash_hex": (
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                ),
            },
        )
        self.assertEqual(request.metadata, {"trace_id": "parent-1", "attempt": 1})
        self.assertIsNotNone(receipt_transport.seen_receipt)
        assert runtime.seen_draft is not None
        self.assertEqual(runtime.seen_draft["caller_ura"], request.caller_ura)
        self.assertEqual(runtime.seen_draft["causal_context"], request.causal_context)
        self.assertEqual(runtime.seen_draft["metadata"], request.metadata)

    def test_child_target_request_inherits_causal_context_and_metadata(self) -> None:
        client = AbilityInvocationClient(
            runtime=RuntimeClient(MemoryRuntimeTransport()),
            addressing=AddressingClient(_identity_transport()),
        )
        child = client.child_context(
            _parent_result_with_receipt(),
            ReceiptClient(ChildReceiptTransport()),
            caller_ura="easynet:///r/example/agent/child.sdk",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            metadata={"trace_id": "parent-1"},
        )

        target = child.target_request(
            ability_ura=ABILITY_URA,
            subject_ura="easynet:///r/example/device/dev-a",
            metadata={"attempt": 2},
        )

        self.assertEqual(target.caller_ura, "easynet:///r/example/agent/child.sdk")
        self.assertEqual(target.metadata, {"trace_id": "parent-1", "attempt": 2})
        self.assertEqual(target.causal_context["form"], "scalar")

    def test_close_delegates_to_owned_clients_once(self) -> None:
        identity = _identity_transport()
        runtime = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime),
            addressing=AddressingClient(identity),
        )

        client.close()
        client.close()

        self.assertEqual(runtime.close_calls, 1)
        self.assertEqual(identity.close_calls, 1)
        with self.assertRaises(SDKError):
            client.invoke(_request(ability_ura=ABILITY_URA))


class ChildReceiptTransport:
    def __init__(self) -> None:
        self.seen_receipt: dict[str, object] | None = None

    def causal_ref(self, receipt_json: bytes) -> bytes:
        self.seen_receipt = json.loads(receipt_json.decode("utf-8"))
        return (
            b'{"form":"scalar",'
            b'"receipt_ura":"easynet:///r/example/receipt/parent-1",'
            b'"receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
            b'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}'
        )


def _identity_transport() -> MemoryIdentityTransport:
    transport = MemoryIdentityTransport()
    transport.identity_json = (
        b'{"kind":"ability","valid":true,'
        b'"ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
        b'"profile":"easynet-strict-v2",'
        b'"components":{"owner_ura":"easynet:///r/example/device/dev-a",'
        b'"owner_kind":"device","public_name":"observe.health",'
        b'"local_registry_ability":"easynet:///r/example/device/dev-a:observe.health",'
        b'"namespace":"observe","local_name":"health"},'
        b'"metadata":{"grammar_owner":"axon"}}'
    )
    transport.descriptor_json = (
        b'{"kind":"descriptor_ref","valid":true,'
        b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",'
        b'"ability_ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
        b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
        b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
        b'"metadata":{"grammar_owner":"axon"}}'
    )
    return transport


def _request(
    *,
    descriptor_ref: str = "",
    ability_ura: str = "",
) -> AbilityCallRequest:
    return _request_with(
        descriptor_ref=descriptor_ref,
        ability_ura=ability_ura,
    )


def _target_request(
    *,
    descriptor_ref: str = "",
    ability_ura: str = "",
) -> AbilityTargetRequest:
    return AbilityTargetRequest(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        descriptor_ref=descriptor_ref,
        ability_ura=ability_ura,
        args={"ready": True},
    )


def _request_with(
    *,
    caller_ura: str = "easynet:///r/example/agent/alice.sdk",
    callee_ura: str = "easynet:///r/example/device/dev-a",
    subject_ura: str = "easynet:///r/example/device/dev-a",
    nonce_base64: str = "AQIDBAUGBwgJCgsMDQ4PEA==",
    content_type: str = "application/json",
    descriptor_ref: str = "",
    ability_ura: str = "",
) -> AbilityCallRequest:
    return AbilityCallRequest(
        caller_ura=caller_ura,
        callee_ura=callee_ura,
        subject_ura=subject_ura,
        nonce_base64=nonce_base64,
        causal_context={"form": "none"},
        content_type=content_type,
        descriptor_ref=descriptor_ref,
        ability_ura=ability_ura,
        args={"city": "Singapore"},
    )


def _parent_result_with_receipt() -> InvocationResult:
    return InvocationResult.from_json(
        json.dumps(
            {
                "ok": True,
                "tuple": {
                    "caller_ura": "easynet:///r/example/agent/alice.sdk",
                    "callee_ura": "easynet:///r/example/device/dev-a",
                    "descriptor_ref": (
                        "easynet:///r/example/ability/"
                        "device.dev-a.observe.health@1.0.0"
                    ),
                    "subject_ura": "easynet:///r/example/device/dev-a",
                    "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                    "causal_context": {"form": "none"},
                    "content_type": "application/json",
                    "args": {},
                },
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_base64": "e30=",
                "output_json": {},
                "elapsed_ms": 8,
                "receipt": {
                    "receipt_ura": "easynet:///r/example/receipt/parent-1",
                    "invocation_id": "inv-parent-1",
                    "self_hash_hex": (
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    ),
                },
                "error": None,
            },
            separators=(",", ":"),
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    unittest.main()
