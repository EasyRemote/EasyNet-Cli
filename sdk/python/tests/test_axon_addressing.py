import json
import unittest
from pathlib import Path

from easynet_sdk import (
    ErrorCode,
    SDKError,
    SdkEnvironment,
    is_code,
)


class AxonAddressingProviderTests(unittest.TestCase):
    def test_shared_projection_corpus(self) -> None:
        fixture_path = (
            Path(__file__).resolve().parents[2]
            / "conformance"
            / "fixtures"
            / "canonical-addressing.v5.json"
        )
        fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
        environment = SdkEnvironment()
        addressing = environment.addressing_client()

        for case in fixture["ura_cases"]:
            request = case["request"]
            kind = request["kind"]
            with self.subTest(name=case["name"]):
                if kind == "user":
                    built = addressing.user_ura(
                        request["realm"], request["user_id"]
                    )
                elif kind == "device":
                    built = addressing.device_ura(
                        request["realm"], request["device_id"]
                    )
                elif kind == "agent":
                    if request["owner_kind"] == "device":
                        built = addressing.device_agent_ura(
                            request["realm"],
                            request["device_id"],
                            request["agent_id"],
                        )
                    else:
                        built = addressing.agent_ura(
                            request["realm"],
                            request["user_id"],
                            request["agent_id"],
                        )
                elif kind == "hub":
                    built = addressing.hub_ura(request["realm"])
                elif kind == "ability":
                    built = addressing.owner_ability_ura(
                        request["owner_ura"], request["ability_name"]
                    )
                elif kind == "resource":
                    built = addressing.resource_ura(
                        request["owner_ura"], request["path"]
                    )
                else:  # pragma: no cover - fixture schema is closed
                    self.fail(f"unsupported fixture kind {kind!r}")
                self.assertEqual(built, case["ura"])
                projection = addressing.parse_ura(built)
                self.assertEqual(projection.profile, fixture["profile"])
                self.assertEqual(projection.components, case["components"])
                self.assertEqual(
                    projection.metadata["grammar_owner"],
                    fixture["grammar_owner"],
                )

        descriptor = addressing.project_descriptor_ref(
            fixture["descriptor"]["raw"]
        )
        self.assertEqual(
            descriptor.components, fixture["descriptor"]["components"]
        )
        for raw in fixture["invalid_uras"]:
            with self.assertRaises(SDKError):
                addressing.parse_ura(raw)
        environment.close()

    def test_environment_provider_builds_descriptor_and_subject(self) -> None:
        environment = SdkEnvironment()
        addressing = environment.addressing_client()

        self.assertEqual(
            addressing.user_ura("example", "alice"),
            "easynet:///r/example/user/alice",
        )

        self.assertEqual(
            addressing.owner_ability_descriptor_ref(
                "easynet:///r/example/device/dev-a",
                "observe.health",
                "1.0.0",
            ),
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(
            addressing.descriptor_bound_resource_subject_ura(
                "easynet:///r/example/user/alice",
                "invoke/observe.health",
            ),
            "easynet:///r/example/resource/user.alice/invoke/observe.health",
        )
        descriptor = addressing.project_descriptor_ref(
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
        )
        self.assertEqual(descriptor.profile, "easynet-strict-v2")
        self.assertEqual(
            descriptor.components,
            {
                "ability_ura": (
                    "easynet:///r/example/ability/device.dev-a.observe.health"
                ),
                "descriptor_version": "1.0.0",
                "owner_ura": "easynet:///r/example/device/dev-a",
                "owner_kind": "device",
                "public_name": "observe.health",
                "local_registry_ability": "observe.health",
            },
        )
        environment.close()

    def test_provider_rejects_non_publisher(self) -> None:
        environment = SdkEnvironment()
        addressing = environment.addressing_client()

        with self.assertRaises(SDKError) as caught:
            addressing.owner_ability_ura(
                "easynet:///r/example/user/alice",
                "observe.health",
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        environment.close()

    def test_project_ability_ura_returns_canonical_projection(self) -> None:
        environment = SdkEnvironment()
        addressing = environment.addressing_client()

        projection = addressing.project_ability_ura(
            "easynet:///r/example/ability/device.dev-a.observe.health"
        )

        self.assertEqual(projection.kind, "ability")
        self.assertEqual(
            projection.ura,
            "easynet:///r/example/ability/device.dev-a.observe.health",
        )
        self.assertEqual(
            projection.owner_ura,
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(projection.public_name, "observe.health")
        environment.close()

    def test_project_ability_ura_rejects_other_ura_kinds(self) -> None:
        environment = SdkEnvironment()
        addressing = environment.addressing_client()

        with self.assertRaises(SDKError) as caught:
            addressing.project_ability_ura(
                "easynet:///r/example/device/dev-a"
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        environment.close()

    def test_provider_rejects_noncanonical_identity_tails(self) -> None:
        environment = SdkEnvironment()
        addressing = environment.addressing_client()

        for raw in (
            "easynet:///r/example/device/dev/a",
            "easynet:///r/example/user/alice.extra",
            "easynet:///r/example/agent/alice.extra.agent",
        ):
            with self.subTest(raw=raw):
                with self.assertRaises(SDKError) as caught:
                    addressing.parse_ura(raw)
                self.assertTrue(
                    is_code(caught.exception, ErrorCode.INVALID_ARGUMENT)
                )
                self.assertEqual(caught.exception.stage, "addressing")

        environment.close()


if __name__ == "__main__":
    unittest.main()
