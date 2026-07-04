import unittest
from unittest.mock import patch

from easynet_sdk import (
    ConnectOptions,
    ErrorCode,
    HealthClient,
    IdentityClient,
    RuntimeClient,
    SDKError,
    SdkEnvironment,
    default_environment,
    is_code,
)
from easynet_sdk._cabi import CLILibrary

from test_cabi import FakeRawCABI


def _load_patch(raw: FakeRawCABI):
    return patch("easynet_sdk._cabi.CLILibrary.load", return_value=CLILibrary(raw))


class SdkEnvironmentTests(unittest.TestCase):
    def test_feature_set_uses_private_runtime_boundary(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = default_environment()
            features = env.feature_set()
            env.close()

        self.assertEqual(features.abi_version, 4)
        self.assertEqual(features.sdk_version, "0.91.30")
        self.assertTrue(features.axon_pb)
        self.assertEqual(raw.shutdown_handles, [])

    def test_connect_local_returns_public_runtime_client(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            client = env.connect_local(ConnectOptions(control_path="/tmp/control.json"))
            self.assertIsInstance(client, RuntimeClient)
            env.close()

        self.assertEqual(raw.daemon_discovers, [{"control_path": "/tmp/control.json"}])
        self.assertEqual(
            raw.daemon_attaches,
            [
                {
                    "control_endpoint": "unix:///tmp/control.sock",
                    "control_path": "/tmp/control.json",
                    "invocation_endpoint": "unix:///tmp/daemon.sock",
                }
            ],
        )
        self.assertEqual(raw.daemon_open_clients, [707])
        self.assertEqual(raw.daemon_detaches, [707])
        self.assertEqual(raw.shutdown_handles, [808])

    def test_identity_helpers_remain_transport_delegated(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            identity = env.identity_client()
            self.assertIsInstance(identity, IdentityClient)
            ability = identity.owner_ability_ura(
                "easynet:///r/example/device/dev-a",
                "observe.health",
            )
            descriptor = identity.canonical_ability_descriptor_ref(ability, "1.0.0")
            env.close()

        self.assertEqual(
            ability,
            "easynet:///r/example/ability/device.dev-a.observe.health",
        )
        self.assertEqual(
            descriptor,
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(
            raw.identity_requests[0],
            (
                "build_ura",
                {
                    "ability_name": "observe.health",
                    "kind": "ability",
                    "owner_ura": "easynet:///r/example/device/dev-a",
                },
            ),
        )
        self.assertEqual(raw.identity_requests[1][0], "project_ura")
        self.assertEqual(raw.identity_requests[2][0], "build_descriptor_ref")
        self.assertEqual(raw.shutdown_handles, [42])

    def test_runtime_and_health_clients_are_environment_owned(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            runtime = env.runtime_client()
            health = env.health_client()
            self.assertIsInstance(runtime, RuntimeClient)
            self.assertIsInstance(health, HealthClient)
            self.assertTrue(health.runtime_health().ready())
            env.close()
            env.close()

        self.assertEqual(raw.runtime_requests, [("health", 42)])
        self.assertEqual(raw.shutdown_handles, [42, 42])

    def test_environment_rejects_use_after_close(self) -> None:
        env = SdkEnvironment()
        env.close()

        with self.assertRaises(SDKError) as raised:
            env.runtime_client()

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_ARGUMENT))


if __name__ == "__main__":
    unittest.main()
