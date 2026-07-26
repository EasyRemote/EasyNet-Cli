import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import easynet_sdk as runtime_sdk
from easynet_sdk import (
    ConnectOptions,
    ErrorCode,
    HealthClient,
    NativeRuntimeHandle,
    RuntimeClient,
    RuntimeAbilityClient,
    SDKError,
    SdkEnvironment,
    default_environment,
    is_code,
)
from easynet_sdk._cabi import EXPECTED_ABI_VERSION, RuntimeCABILibrary
from easynet_sdk.axon_addressing import AddressingClient, AxonAddressingTransport
from easynet_sdk.runtime_ability import RuntimeCallContext

from test_cabi import FakeRawCABI
from test_runtime_ability import RuntimeTransportFake


downstreamProfileSymbols = (
    "WorkflowClient",
    "WorkflowTransport",
    "ApplicationLifecycleClient",
    "ApplicationDirectoryView",
    "ApplicationReceiptPage",
    "ApplicationEventClient",
    "HostIntegrationClient",
    "PublicationWorkflowClient",
    "TranslationLayer",
    "ConvenienceWrapperClient",
    "ProfileBundle",
    "ServiceLocator",
)


def _load_patch(raw: FakeRawCABI):
    return patch(
        "easynet_sdk._cabi.RuntimeCABILibrary.load",
        return_value=RuntimeCABILibrary(raw),
    )


class SdkEnvironmentTests(unittest.TestCase):
    def test_native_runtime_rejects_incomplete_provider_graph(self) -> None:
        with self.assertRaises(SDKError) as caught:
            NativeRuntimeHandle(None, None, None)  # type: ignore[arg-type]

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn("runtime provider is required", caught.exception.message)

    def test_feature_set_reports_generic_cabi_runtime(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = default_environment()
            features = env.feature_set()
            env.close()

        self.assertEqual(features.abi_version, EXPECTED_ABI_VERSION)
        self.assertEqual(features.profiles, {"runtime_core": "provider-backed"})

    def test_native_runtime_owns_runtime_and_health(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            provider = env.native_runtime()

            self.assertIsInstance(provider, NativeRuntimeHandle)
            self.assertIsInstance(provider.client(), RuntimeClient)
            self.assertIsInstance(provider.health(), HealthClient)
            self.assertTrue(provider.health().runtime_health().ready())
            self.assertEqual(
                provider.addressing().owner_ability_descriptor_ref(
                    "easynet:///r/example/device/dev-a",
                    "observe.health",
                    "1.0.0",
                ),
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
            )

            provider.close()
            env.close()

        self.assertEqual(raw.shutdown_handles, [42])

    def test_native_runtime_handle_provides_runtime_ability_facade(self) -> None:
        transport = RuntimeTransportFake()
        runtime = RuntimeClient(transport)  # type: ignore[arg-type]
        health = HealthClient(lambda: b'{"runtime_ready":true,"diagnostics":[]}')
        addressing = AddressingClient(AxonAddressingTransport())
        provider = NativeRuntimeHandle(runtime, health, addressing)

        ability = provider.ability_client()
        self.assertIsInstance(ability, RuntimeAbilityClient)
        draft = ability.build(
            RuntimeCallContext(
                caller_ura="easynet:///r/example/agent/alice.client",
                callee_ura="easynet:///r/example/authority",
                subject_ura="easynet:///r/example/user/alice",
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
            ),
            "namespace.resolve",
            {"name": "alice"},
        )
        self.assertEqual(
            draft.descriptor_ref,
            "easynet:///r/example/ability/authority.namespace.resolve@1.0.0#"
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read",
        )
        self.assertEqual(
            transport.descriptor_requests[-1]["call_mode"],
            "rpc",
        )

        provider.close()
        self.assertTrue(transport.closed)
        with self.assertRaises(SDKError) as raised:
            provider.ability_client()
        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_ARGUMENT))

    def test_connect_local_returns_generic_runtime_client(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            client = env.connect_local(ConnectOptions(control_path="/tmp/control.json"))
            result = client.invoke(_complete_draft())
            env.close()

        self.assertTrue(result.ok)
        self.assertEqual(raw.runtime_host_open_clients, [707])
        self.assertEqual(raw.runtime_host_detaches, [707])
        self.assertEqual(raw.shutdown_handles, [808])

    def test_runtime_connection_uses_generic_cabi_connector_and_closes_runtime(self) -> None:
        raw = FakeRawCABI()
        with tempfile.TemporaryDirectory() as tmp:
            control_path = _write_control_discovery(tmp)
            with _load_patch(raw):
                env = SdkEnvironment(control_path=str(control_path))
                connection = env.runtime_connection()
                result = connection.runtime_client().invoke(_complete_draft())
                env.close()

        self.assertTrue(result.ok)
        self.assertEqual(raw.runtime_host_open_clients, [707])
        self.assertEqual(raw.runtime_host_detaches, [707])
        self.assertEqual(raw.shutdown_handles, [808])

    def test_runtime_control_discovery_reader_uses_generic_projection(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            control_path = _write_control_discovery(tmp)
            discovery = runtime_sdk.read_runtime_control_discovery(control_path)

        self.assertEqual(discovery.socket_path, "/tmp/control.sock")
        self.assertEqual(discovery.invocation_endpoint, "unix:///tmp/runtime-host.sock")
        self.assertEqual(discovery.runtime_host_version, "test")
        self.assertEqual(discovery.supported_ipc_versions.min, 1)
        self.assertEqual(discovery.supported_ipc_versions.max, 1)
        self.assertEqual(discovery.capability_flags, ("invocation",))

    def test_default_environment_exposes_no_product_profiles(self) -> None:
        env = SdkEnvironment()
        for symbol in downstreamProfileSymbols:
            self.assertFalse(hasattr(runtime_sdk, symbol), symbol)
            self.assertFalse(hasattr(env, symbol), symbol)

        addressing = env.addressing_client()
        self.assertEqual(
            addressing.resource_ura(
                "easynet:///r/example/user/alice",
                "invoke/files.read",
            ),
            "easynet:///r/example/resource/user.alice/invoke/files.read",
        )
        env.close()

    def test_environment_rejects_use_after_close(self) -> None:
        env = SdkEnvironment()
        env.close()

        with self.assertRaises(SDKError) as caught:
            env.runtime_client()

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))


def _complete_draft():
    from test_runtime import complete_draft

    return complete_draft()


def _write_control_discovery(directory: str) -> Path:
    path = Path(directory) / "control.json"
    path.write_text(
        '{"socket_path":"/tmp/control.sock",'
        '"invocation_endpoint":"unix:///tmp/runtime-host.sock",'
        '"pid":123,'
        '"daemon_version":"test",'
        '"supported_ipc_versions":{"min":1,"max":1},'
        '"capability_flags":["invocation"]}',
        encoding="utf-8",
    )
    return path


if __name__ == "__main__":
    unittest.main()
