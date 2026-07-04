import unittest

from easynet_sdk import (
    AbilityCallRequest,
    AbilityInvocationClient,
    BidiStreamDescriptor,
    ErrorCode,
    RuntimeClient,
    SDKError,
    is_code,
)
from easynet_sdk.identity import AddressingClient

from test_identity import MemoryIdentityTransport
from test_runtime import MemoryRuntimeTransport


class AbilityInvocationClientTests(unittest.TestCase):
    def test_build_invocation_from_ability_name_delegates_descriptor_ref(self) -> None:
        identity = _identity_transport()
        runtime = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime),
            addressing=AddressingClient(identity),
        )

        draft = client.build_invocation(_request(ability_name="observe.health"))

        self.assertEqual(
            draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(draft.subject_ura, "easynet:///r/example/device/dev-a")
        self.assertEqual(draft.causal_context, {"form": "none"})
        self.assertEqual(
            identity.seen_requests,
            [
                {
                    "kind": "ability",
                    "owner_ura": "easynet:///r/example/device/dev-a",
                    "ability_name": "observe.health",
                },
                {
                    "ability_ura": "easynet:///r/example/ability/device.dev-a.observe.health",
                    "descriptor_version": "1.0.0",
                },
            ],
        )

    def test_build_invocation_from_ability_ura_uses_descriptor_builder(self) -> None:
        identity = _identity_transport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(MemoryRuntimeTransport()),
            addressing=AddressingClient(identity),
        )

        draft = client.build_invocation(
            _request(
                ability_ura="easynet:///r/example/ability/device.dev-a.observe.health"
            )
        )

        self.assertEqual(
            draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(
            identity.seen_requests,
            [
                {
                    "ability_ura": "easynet:///r/example/ability/device.dev-a.observe.health",
                    "descriptor_version": "1.0.0",
                }
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
                    "easynet:///r/example/ability/"
                    "device.dev-a.observe.health@1.0.0"
                )
            )
        )

        self.assertEqual(
            draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(
            identity.seen_requests,
            [
                {
                    "descriptor_ref": (
                        "easynet:///r/example/ability/"
                        "device.dev-a.observe.health@1.0.0"
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

        result = client.invoke(_request(ability_name="observe.health"))
        stream = client.stream(_request(ability_name="observe.health"))
        event = stream.next()
        stream.close()
        bidi = client.bidi(
            _request(ability_name="observe.health"),
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
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(
            runtime_transport.seen_streams,
            [{"content_type": "application/json", "stream_id": 1}],
        )

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
                    ability_name="observe.health",
                    ability_ura=(
                        "easynet:///r/example/ability/"
                        "device.dev-a.observe.health"
                    ),
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
                    ability_name="observe.health",
                )
            )

    def test_rejects_surrounding_whitespace_before_dispatch(self) -> None:
        runtime = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime),
            addressing=AddressingClient(_identity_transport()),
        )

        with self.assertRaises(SDKError) as caught:
            client.invoke(_request(ability_name=" observe.health"))
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

        with self.assertRaises(SDKError) as caught:
            client.invoke(
                _request_with(
                    caller_ura=" easynet:///r/example/agent/alice.sdk",
                    ability_name="observe.health",
                )
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(runtime.seen_draft)

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
            client.invoke(_request(ability_name="observe.health"))


def _identity_transport() -> MemoryIdentityTransport:
    transport = MemoryIdentityTransport()
    transport.identity_json = (
        b'{"kind":"ability","valid":true,'
        b'"ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
        b'"profile":"easynet-strict-v2",'
        b'"components":{"owner_ura":"easynet:///r/example/device/dev-a",'
        b'"public_name":"observe.health"},'
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
    ability_name: str = "",
) -> AbilityCallRequest:
    return _request_with(
        descriptor_ref=descriptor_ref,
        ability_ura=ability_ura,
        ability_name=ability_name,
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
    ability_name: str = "",
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
        ability_name=ability_name,
        args={"city": "Singapore"},
    )


if __name__ == "__main__":
    unittest.main()
