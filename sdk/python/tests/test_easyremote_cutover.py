import unittest

from easynet_sdk import AddressingClient, DaemonInvocationTransport, RuntimeClient

from test_identity import MemoryIdentityTransport
from test_runtime import MemoryRuntimeTransport


class EasyRemoteCutoverTests(unittest.TestCase):
    def test_easyremote_style_unary_invoke_uses_sdk_addressing_and_transport(
        self,
    ) -> None:
        identity_transport = MemoryIdentityTransport()
        identity_transport.identity_json = (
            b'{"kind":"ability","valid":true,'
            b'"ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a",'
            b'"public_name":"er.weather"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        identity_transport.descriptor_json = (
            b'{"kind":"descriptor_ref","valid":true,'
            b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",'
            b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        addressing = AddressingClient(identity_transport)
        runtime_transport = MemoryRuntimeTransport()
        daemon_transport = DaemonInvocationTransport.from_runtime_client(
            RuntimeClient(runtime_transport)
        )

        descriptor_ref = addressing.owner_ability_descriptor_ref(
            "easynet:///r/example/device/dev-a",
            "er.weather",
            "1.0.0",
        )
        result = daemon_transport.invoke(
            {
                "caller_ura": "easynet:///r/example/agent/alice.sdk",
                "callee_ura": "easynet:///r/example/device/dev-a",
                "descriptor_ref": descriptor_ref,
                "subject_ura": "easynet:///r/example/device/dev-a",
                "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                "causal_context": {"form": "none"},
                "content_type": "application/json",
                "args": {"city": "Singapore"},
            }
        )

        self.assertTrue(result["ok"])
        self.assertEqual(
            descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
        )
        self.assertEqual(
            identity_transport.seen_requests,
            [
                {
                    "kind": "ability",
                    "owner_ura": "easynet:///r/example/device/dev-a",
                    "ability_name": "er.weather",
                },
                {
                    "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                    "descriptor_version": "1.0.0",
                },
            ],
        )
        assert runtime_transport.seen_draft is not None
        self.assertEqual(runtime_transport.seen_draft["descriptor_ref"], descriptor_ref)
        self.assertEqual(runtime_transport.seen_draft["args"], {"city": "Singapore"})


if __name__ == "__main__":
    unittest.main()
