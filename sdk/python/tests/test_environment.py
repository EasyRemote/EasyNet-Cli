import unittest
from unittest.mock import patch

from easynet_sdk import (
    AbilityDeployRequest,
    AddressingClient,
    AdminAgentListRequest,
    AdminCarrierBase,
    CompatibilityCarrierBase,
    CompatibilityFileDeleteRequest,
    CompatibilityFileRequest,
    CompatibilityFileUploadRequest,
    CompatibilityListModelsRequest,
    ConnectOptions,
    DaemonInvocationTransport,
    DeviceQuery,
    DirectoryQueryBase,
    ErrorCode,
    EventsCarrierBase,
    EventsSubscriptionRequest,
    HealthClient,
    HostStreamBindingRequest,
    IdentityClient,
    LocalResourceRefRequest,
    MissionCarrierBase,
    MissionRunRequest,
    ReceiptFetchRequest,
    RuntimeClient,
    SDKError,
    SdkEnvironment,
    SurfaceCarrierBase,
    SurfaceListPagesRequest,
    WrapperBrowserSessionRequest,
    WrapperBrowserStartRequest,
    WrapperCarrierBase,
    WrapperFileRecordRequest,
    WrapperFileTransferRequest,
    WrapperMediaSessionRequest,
    WrapperMediaStartRequest,
    WrapperRemoteDesktopSessionRequest,
    WrapperRemoteDesktopStartRequest,
    WrapperTerminalSessionRequest,
    WrapperTerminalStartRequest,
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

    def test_addressing_client_exposes_narrow_identity_helper_surface(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            addressing = env.addressing_client()
            self.assertIsInstance(addressing, AddressingClient)
            ability = addressing.owner_ability_ura(
                "easynet:///r/example/device/dev-a",
                "observe.health",
            )
            descriptor = addressing.canonical_ability_descriptor_ref(
                ability,
                "1.0.0",
            )
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
            [entry[0] for entry in raw.identity_requests],
            ["build_ura", "project_ura", "build_descriptor_ref"],
        )
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

    def test_invocation_transport_is_environment_owned(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            transport = env.invocation_transport()
            self.assertIsInstance(transport, DaemonInvocationTransport)
            result = transport.invoke(
                {
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
                }
            )
            env.close()

        self.assertTrue(result["ok"])
        self.assertEqual(result["terminal_state"], "Completed")
        self.assertEqual(raw.shutdown_handles, [42])

    def test_profile_factories_delegate_carriers_to_private_cabi(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            receipt = env.receipt_client()
            publication = env.publication_client()
            host = env.host_binding_client()
            mission = env.mission_client()
            admin = env.admin_client()
            events = env.event_client()
            directory = env.directory_client()
            surface = env.surface_client()
            compatibility = env.compatibility_client()
            wrapper = env.wrapper_client()

            self.assertTrue(receipt.verify(b'{"receipt_ura":"receipt-1"}').verified)
            resource_ref = publication.build_local_resource_ref(
                LocalResourceRefRequest(path="/tmp/package", capability="read")
            )
            deploy_request = AbilityDeployRequest(
                caller_ura=_CALLER,
                callee_ura=_CALLEE,
                subject_ura=_SUBJECT,
                descriptor_version="1.0.0",
                nonce_base64=_NONCE,
                causal_context=_CAUSAL,
                resource_ref=resource_ref,
                node_id="local",
            )
            publication.build_deploy_invocation(deploy_request)
            binding = host.build_host_stream_binding(
                HostStreamBindingRequest(
                    binding_id="binding-weather-1",
                    descriptor_ref=(
                        "easynet:///r/example/ability/"
                        "device.dev-a.weather.stream@1.0.0"
                    ),
                    endpoint="/tmp/easynet-weather.sock",
                )
            )
            mission.build_run_eal_invocation(
                MissionRunRequest(base=_mission_base(), source="mission weather")
            )
            admin.build_agent_list_invocation(AdminAgentListRequest(_admin_base()))
            events.build_directory_subscription_invocation(
                EventsSubscriptionRequest(base=_events_base())
            )
            directory.build_list_devices_invocation(DeviceQuery(_directory_base()))
            surface.build_list_pages_invocation(
                SurfaceListPagesRequest(base=_surface_base())
            )
            compatibility.build_list_models_invocation(
                CompatibilityListModelsRequest(base=_compatibility_base())
            )
            compatibility.build_file_upload_invocation(
                CompatibilityFileUploadRequest(
                    base=_compatibility_base(),
                    purpose="batch",
                    id="file-easynet-docs-1",
                    file_ref=(
                        "easynet:///r/example/resource/alice.files/prompt.jsonl"
                    ),
                    filename="prompt.jsonl",
                )
            )
            compatibility.build_file_retrieve_invocation(
                CompatibilityFileRequest(
                    base=_compatibility_base(),
                    id="file-easynet-docs-1",
                    file_ref=(
                        "easynet:///r/example/resource/alice.files/prompt.jsonl"
                    ),
                    filename="prompt.jsonl",
                )
            )
            compatibility.build_file_delete_invocation(
                CompatibilityFileDeleteRequest(
                    base=_compatibility_base(),
                    id="file-easynet-docs-1",
                    file_ref=(
                        "easynet:///r/example/resource/alice.files/prompt.jsonl"
                    ),
                    deleted=True,
                )
            )
            wrapper.build_file_transfer_invocation(
                WrapperFileTransferRequest(
                    base=_wrapper_base(),
                    file=WrapperFileRecordRequest(
                        file_ref=(
                            "easynet:///r/example/resource/alice.files/report.txt"
                        ),
                        owner_ura=_CALLER,
                        content_type="text/plain",
                        size_bytes=42,
                    ),
                )
            )
            wrapper.build_terminal_session_invocation(
                WrapperTerminalStartRequest(
                    base=_wrapper_base(),
                    session=WrapperTerminalSessionRequest(
                        session_id="term-1",
                        owner_ura=_CALLER,
                        state="starting",
                    ),
                    command=("bash", "-lc"),
                )
            )
            wrapper.build_remote_desktop_session_invocation(
                WrapperRemoteDesktopStartRequest(
                    base=_wrapper_base(),
                    session=WrapperRemoteDesktopSessionRequest(
                        session_id="rdp-1",
                        owner_ura=_CALLER,
                        state="starting",
                        display_ref="display-main",
                    ),
                    display="main",
                )
            )
            wrapper.build_browser_session_invocation(
                WrapperBrowserStartRequest(
                    base=_wrapper_base(),
                    session=WrapperBrowserSessionRequest(
                        session_id="browser-1",
                        owner_ura=_CALLER,
                        state="starting",
                        browser_ref="browser-main",
                    ),
                    url="https://example.com",
                )
            )
            wrapper.build_media_session_invocation(
                WrapperMediaStartRequest(
                    base=_wrapper_base(),
                    session=WrapperMediaSessionRequest(
                        session_id="media-1",
                        owner_ura=_CALLER,
                        state="starting",
                        media_kind="voice",
                        stream_ref="stream-voice-1",
                    ),
                    codec="opus",
                )
            )
            receipt.build_fetch_invocation(
                ReceiptFetchRequest(
                    caller_ura=_CALLER,
                    callee_ura=_CALLEE,
                    descriptor_ref=(
                        "easynet:///r/example/ability/"
                        "device.dev-a.invocation.history.get@1.0.0"
                    ),
                    subject_ura=_SUBJECT,
                    descriptor_version="1.0.0",
                    nonce_base64=_NONCE,
                    causal_context=_CAUSAL,
                    request_id="inv-example-1",
                )
            )
            env.close()

        self.assertEqual(binding.lifecycle["frame_contract_owner"], "daemon_sdk")
        symbols = [item[0] for item in raw.profile_requests]
        self.assertIn("easynet_receipt_verify", symbols)
        self.assertIn("easynet_receipt_build_fetch_invocation", symbols)
        self.assertIn("easynet_publication_build_resource_ref", symbols)
        self.assertIn("easynet_publication_build_deploy_invocation", symbols)
        self.assertIn("easynet_host_binding_build", symbols)
        self.assertIn("easynet_mission_build_run_eal_invocation", symbols)
        self.assertIn("easynet_admin_build_agent_list_invocation", symbols)
        self.assertIn("easynet_events_build_directory_subscription_invocation", symbols)
        self.assertIn("easynet_directory_build_list_devices_invocation", symbols)
        self.assertIn("easynet_surface_build_list_pages_invocation", symbols)
        self.assertIn("easynet_compatibility_build_list_models_invocation", symbols)
        self.assertIn("easynet_compatibility_build_file_upload_invocation", symbols)
        self.assertIn("easynet_compatibility_build_file_retrieve_invocation", symbols)
        self.assertIn("easynet_compatibility_build_file_delete_invocation", symbols)
        self.assertIn("easynet_wrappers_build_file_transfer_invocation", symbols)
        self.assertIn("easynet_wrappers_build_terminal_session_invocation", symbols)
        self.assertIn(
            "easynet_wrappers_build_remote_desktop_session_invocation", symbols
        )
        self.assertIn("easynet_wrappers_build_browser_session_invocation", symbols)
        self.assertIn("easynet_wrappers_build_media_session_invocation", symbols)

    def test_missing_live_profile_operations_are_typed_not_implemented(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            publication = env.publication_client()
            resource_ref = publication.build_local_resource_ref(
                LocalResourceRefRequest(path="/tmp/package", capability="read")
            )

            with self.assertRaises(SDKError) as raised:
                publication.deploy_ability(
                    AbilityDeployRequest(
                        caller_ura=_CALLER,
                        callee_ura=_CALLEE,
                        subject_ura=_SUBJECT,
                        descriptor_version="1.0.0",
                        nonce_base64=_NONCE,
                        causal_context=_CAUSAL,
                        resource_ref=resource_ref,
                        node_id="local",
                    )
                )
            env.close()

        self.assertTrue(is_code(raised.exception, ErrorCode.NOT_IMPLEMENTED))

    def test_environment_rejects_use_after_close(self) -> None:
        env = SdkEnvironment()
        env.close()

        with self.assertRaises(SDKError) as raised:
            env.runtime_client()

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_ARGUMENT))

_CALLER = "easynet:///r/example/agent/alice.sdk"
_CALLEE = "easynet:///r/example/device/dev-a"
_SUBJECT = "easynet:///r/example/device/dev-a"
_NONCE = "AQIDBAUGBwgJCgsMDQ4PEA=="
_CAUSAL = {"form": "none"}


def _admin_base() -> AdminCarrierBase:
    return AdminCarrierBase(
        caller_ura=_CALLER,
        callee_ura=_CALLEE,
        subject_ura=_SUBJECT,
        descriptor_version="1.0.0",
        nonce_base64=_NONCE,
        causal_context=_CAUSAL,
    )


def _mission_base() -> MissionCarrierBase:
    return MissionCarrierBase(
        caller_ura=_CALLER,
        callee_ura=_CALLEE,
        subject_ura=_SUBJECT,
        descriptor_version="1.0.0",
        nonce_base64=_NONCE,
        causal_context=_CAUSAL,
    )


def _events_base() -> EventsCarrierBase:
    return EventsCarrierBase(
        caller_ura=_CALLER,
        callee_ura=_CALLEE,
        subject_ura=_SUBJECT,
        descriptor_version="1.0.0",
        nonce_base64=_NONCE,
        causal_context=_CAUSAL,
    )


def _directory_base() -> DirectoryQueryBase:
    return DirectoryQueryBase(
        caller_ura=_CALLER,
        callee_ura=_CALLEE,
        subject_ura=_SUBJECT,
        descriptor_version="1.0.0",
        nonce_base64=_NONCE,
        causal_context=_CAUSAL,
    )


def _surface_base() -> SurfaceCarrierBase:
    return SurfaceCarrierBase(
        caller_ura=_CALLER,
        callee_ura=_CALLEE,
        subject_ura=_SUBJECT,
        descriptor_version="1.0.0",
        nonce_base64=_NONCE,
        causal_context=_CAUSAL,
    )


def _compatibility_base() -> CompatibilityCarrierBase:
    return CompatibilityCarrierBase(
        caller_ura=_CALLER,
        callee_ura=_CALLEE,
        subject_ura=_SUBJECT,
        descriptor_version="1.0.0",
        nonce_base64=_NONCE,
        causal_context=_CAUSAL,
    )


def _wrapper_base() -> WrapperCarrierBase:
    return WrapperCarrierBase(
        caller_ura=_CALLER,
        callee_ura=_CALLEE,
        subject_ura=_SUBJECT,
        descriptor_version="1.0.0",
        nonce_base64=_NONCE,
        causal_context=_CAUSAL,
    )


if __name__ == "__main__":
    unittest.main()
