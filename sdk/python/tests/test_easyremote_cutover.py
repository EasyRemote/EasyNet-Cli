import unittest

from easynet_sdk import (
    AbilityCallRequest,
    AbilityInvocationClient,
    AbilityTargetRequest,
    AddressingClient,
    RuntimeClient,
    ability_address,
)

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
            b'"owner_kind":"device","public_name":"er.weather",'
            b'"local_registry_ability":"easynet:///r/example/device/dev-a:er.weather",'
            b'"namespace":"er","local_name":"weather"},'
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
        runtime_transport = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime_transport),
            addressing=AddressingClient(identity_transport),
        )
        address = client.addressing.ability_address(
            "easynet:///r/example/ability/device.dev-a.er.weather"
        )

        result = client.invoke(
            AbilityCallRequest(
                caller_ura="easynet:///r/example/agent/alice.sdk",
                callee_ura=address.owner_ura,
                subject_ura=address.subject_ura,
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                ability_ura=address.ability_ura,
                args={"city": "Singapore"},
            )
        )

        self.assertTrue(result.ok)
        self.assertEqual(address.public_name, "er.weather")
        self.assertEqual(address.owner_kind, "device")
        self.assertEqual(address.namespace, "er")
        self.assertEqual(address.local_name, "weather")
        self.assertEqual(
            identity_transport.seen_requests,
            [
                {"ura": "easynet:///r/example/ability/device.dev-a.er.weather"},
                {
                    "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                    "descriptor_version": "1.0.0",
                },
            ],
        )
        assert runtime_transport.seen_draft is not None
        self.assertEqual(
            runtime_transport.seen_draft["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
        )
        self.assertEqual(runtime_transport.seen_draft["args"], {"city": "Singapore"})

    def test_package_level_ability_address_uses_default_sdk_facade(self) -> None:
        from test_environment import FakeRawCABI, _load_patch

        raw = FakeRawCABI()
        with _load_patch(raw):
            address = ability_address(
                "easynet:///r/example/ability/device.dev-a.observe.health",
                control_path="/tmp/control.json",
            )

        self.assertEqual(
            address.ability_ura,
            "easynet:///r/example/ability/device.dev-a.observe.health",
        )
        self.assertEqual(address.subject_ura, address.ability_ura)
        self.assertEqual(address.owner_ura, "easynet:///r/example/device/dev-a")
        self.assertEqual(address.owner_kind, "device")
        self.assertEqual(address.public_name, "observe.health")
        self.assertEqual(raw.init_paths, ["/tmp/control.json"])
        self.assertEqual(raw.shutdown_handles, [42])
        self.assertEqual([entry[0] for entry in raw.identity_requests], ["project_ura"])

    def test_easyremote_style_target_invoke_uses_sdk_target_facade(self) -> None:
        identity_transport = MemoryIdentityTransport()
        identity_transport.identity_json = (
            b'{"kind":"ability","valid":true,'
            b'"ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a",'
            b'"owner_kind":"device","public_name":"er.weather",'
            b'"local_registry_ability":"easynet:///r/example/device/dev-a:er.weather",'
            b'"namespace":"er","local_name":"weather"},'
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
        runtime_transport = MemoryRuntimeTransport()
        client = AbilityInvocationClient(
            runtime=RuntimeClient(runtime_transport),
            addressing=AddressingClient(identity_transport),
        )

        result = client.invoke_target(
            AbilityTargetRequest(
                caller_ura="easynet:///r/example/agent/alice.sdk",
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                ability_ura="easynet:///r/example/ability/device.dev-a.er.weather",
                args={"city": "Singapore"},
            )
        )

        self.assertTrue(result.ok)
        assert runtime_transport.seen_draft is not None
        self.assertEqual(
            runtime_transport.seen_draft["callee_ura"],
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(
            runtime_transport.seen_draft["subject_ura"],
            "easynet:///r/example/ability/device.dev-a.er.weather",
        )
        self.assertEqual(
            runtime_transport.seen_draft["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
        )


if __name__ == "__main__":
    unittest.main()
