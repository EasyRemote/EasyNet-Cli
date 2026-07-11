import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from easynet_sdk import (
    AbilityDeployRequest,
    AbilityCallRequest,
    AbilityInvocationClient,
    AdminClient,
    AddressingClient,
    AdminAgentListRequest,
    AdminAgentStartRequest,
    AdminCarrierBase,
    CompatibilityCarrierBase,
    CompatibilityChatCompletionRequest,
    CompatibilityFileDeleteRequest,
    CompatibilityFileRequest,
    CompatibilityFileUploadRequest,
    CompatibilityListModelsRequest,
    ConnectOptions,
    ConnectionState,
    DaemonInvocationTransport,
    DeviceQuery,
    DirectoryClient,
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
    NativeRuntimeHandle,
    PublishedAbilityQuery,
    ReceiptClient,
    ReceiptFetchRequest,
    RuntimeClient,
    RuntimeConnection,
    SDKError,
    SdkEnvironment,
    SurfaceCarrierBase,
    SurfaceCreatePageRequest,
    SurfaceListPagesRequest,
    SurfaceManifestRequest,
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
    AttachOptions,
    ability_ura_from_descriptor_ref,
    canonical_ability_descriptor_ref,
    default_environment,
    is_code,
    owner_ability_descriptor_ref,
    owner_ability_ura,
    owner_ura_for_ability,
    parse_ura,
    project_descriptor_ref,
)
from easynet_sdk._cabi import CLILibrary

from test_cabi import FakeRawCABI


def _load_patch(raw: FakeRawCABI):
    return patch("easynet_sdk._cabi.CLILibrary.load", return_value=CLILibrary(raw))


class SdkEnvironmentTests(unittest.TestCase):
    def test_native_runtime_owns_runtime_health_and_identity(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            provider = env.native_runtime()
            self.assertIsInstance(provider, NativeRuntimeHandle)
            self.assertIsInstance(provider.client(), RuntimeClient)
            self.assertIsInstance(provider.health(), HealthClient)
            self.assertIsInstance(provider.identity(), IdentityClient)
            self.assertTrue(provider.health().runtime_health().ready())

            provider.close()
            with self.assertRaises(SDKError) as caught:
                provider.identity()
            self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
            env.close()

        # Runtime and identity own distinct C ABI handles. Health borrows the
        # Runtime handle, so provider close must not double-close it.
        self.assertEqual(raw.shutdown_handles, [42, 42])

    def test_native_runtime_identity_open_failure_releases_runtime(self) -> None:
        raw = FakeRawCABI()
        with (
            _load_patch(raw),
            patch(
                "easynet_sdk._cabi.open_cabi_identity_transport",
                side_effect=OSError("identity unavailable"),
            ),
        ):
            env = SdkEnvironment(control_path="/tmp/control.json")
            with self.assertRaises(OSError):
                env.native_runtime()
            env.close()

        self.assertEqual(raw.shutdown_handles, [42])

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

    def test_runtime_connection_uses_cabi_connector_and_closes_runtime(self) -> None:
        raw = FakeRawCABI()
        with tempfile.TemporaryDirectory() as tmp:
            control_path = _write_control_discovery(tmp)
            with _load_patch(raw):
                env = SdkEnvironment(control_path=str(control_path))
                connection = env.runtime_connection()
                self.assertIsInstance(connection, RuntimeConnection)
                self.assertEqual(connection.state, ConnectionState.READY)
                assert connection.endpoint is not None
                self.assertEqual(connection.endpoint.endpoint, f"{tmp}/daemon.sock")
                self.assertEqual(connection.endpoint.control_endpoint, f"{tmp}/control.sock")
                client = connection.runtime_client()
                self.assertIsInstance(client, RuntimeClient)
                env.close()

        self.assertEqual(raw.daemon_discovers, [])
        self.assertEqual(raw.daemon_open_clients, [707])
        self.assertEqual(raw.daemon_detaches, [707])
        self.assertEqual(raw.shutdown_handles, [808])

    def test_runtime_connection_resolves_default_control_path(self) -> None:
        raw = FakeRawCABI()
        with tempfile.TemporaryDirectory() as tmp:
            control_path = _write_control_discovery(tmp)
            with (
                _load_patch(raw),
                patch(
                    "easynet_sdk.environment.default_control_path",
                    return_value=control_path,
                ),
            ):
                env = SdkEnvironment()
                self.assertEqual(env.resolved_control_path(), str(control_path))
                connection = env.runtime_connection()
                assert connection.endpoint is not None
                self.assertEqual(connection.endpoint.control_path, str(control_path))
                env.close()

        self.assertEqual(raw.daemon_open_clients, [707])
        self.assertEqual(raw.daemon_detaches, [707])
        self.assertEqual(raw.shutdown_handles, [808])

    def test_connect_options_control_path_overrides_environment_default(self) -> None:
        raw = FakeRawCABI()
        with tempfile.TemporaryDirectory() as tmp:
            default_control = Path(tmp) / "default.json"
            override_control = _write_control_discovery(tmp)
            with (
                _load_patch(raw),
                patch(
                    "easynet_sdk.environment.default_control_path",
                    return_value=default_control,
                ),
            ):
                env = SdkEnvironment()
                connection = env.runtime_connection(
                    ConnectOptions(control_path=str(override_control))
                )
                assert connection.endpoint is not None
                self.assertEqual(
                    connection.endpoint.control_path,
                    str(override_control),
                )
                env.close()

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

    def test_package_addressing_helpers_use_default_cabi_facade(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            parsed = parse_ura(
                "easynet:///r/example/ability/device.dev-a.observe.health",
                control_path="/tmp/control.json",
            )
            ability = owner_ability_ura(
                "easynet:///r/example/device/dev-a",
                "observe.health",
                control_path="/tmp/control.json",
            )
            owner = owner_ura_for_ability(
                ability,
                control_path="/tmp/control.json",
            )
            descriptor = canonical_ability_descriptor_ref(
                ability,
                "1.0.0",
                control_path="/tmp/control.json",
            )
            ability_from_ref = ability_ura_from_descriptor_ref(
                descriptor,
                control_path="/tmp/control.json",
            )
            projection = project_descriptor_ref(
                descriptor,
                control_path="/tmp/control.json",
            )
            owner_descriptor = owner_ability_descriptor_ref(
                "easynet:///r/example/device/dev-a",
                "observe.health",
                "1.0.0",
                control_path="/tmp/control.json",
            )

        self.assertEqual(parsed.kind, "ability")
        self.assertEqual(
            ability,
            "easynet:///r/example/ability/device.dev-a.observe.health",
        )
        self.assertEqual(owner, "easynet:///r/example/device/dev-a")
        self.assertEqual(
            descriptor,
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(ability_from_ref, ability)
        self.assertEqual(projection.descriptor_ref, descriptor)
        self.assertEqual(projection.ability_ura, ability)
        self.assertEqual(projection.descriptor_version, "1.0.0")
        self.assertEqual(owner_descriptor, descriptor)
        self.assertEqual(raw.init_paths, ["/tmp/control.json"] * 7)
        self.assertEqual(raw.shutdown_handles, [42] * 7)
        self.assertEqual(
            [entry[0] for entry in raw.identity_requests],
            [
                "project_ura",
                "build_ura",
                "project_ura",
                "project_ura",
                "build_descriptor_ref",
                "project_descriptor_ref",
                "project_descriptor_ref",
                "build_ura",
                "project_ura",
                "build_descriptor_ref",
            ],
        )

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

    def test_clients_resolve_default_control_path_before_cabi_open(self) -> None:
        raw = FakeRawCABI()
        with (
            _load_patch(raw),
            patch(
                "easynet_sdk.environment.default_control_path",
                return_value=Path("/tmp/default-control.json"),
            ),
        ):
            env = SdkEnvironment()
            runtime = env.runtime_client()
            identity = env.identity_client()
            self.assertIsInstance(runtime, RuntimeClient)
            self.assertIsInstance(identity, IdentityClient)
            env.close()

        self.assertEqual(
            raw.init_paths,
            ["/tmp/default-control.json", "/tmp/default-control.json"],
        )
        self.assertEqual(raw.shutdown_handles, [42, 42])

    def test_invocation_transport_is_environment_owned(self) -> None:
        raw = FakeRawCABI()
        with tempfile.TemporaryDirectory() as tmp:
            control_path = _write_control_discovery(tmp)
            with _load_patch(raw):
                env = SdkEnvironment(control_path=str(control_path))
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
        self.assertEqual(raw.daemon_discovers, [])
        self.assertEqual(raw.daemon_open_clients, [707])
        self.assertEqual(raw.daemon_detaches, [707])
        self.assertEqual(raw.shutdown_handles, [808])

    def test_ability_invocation_client_uses_cabi_runtime_and_identity(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            client = env.ability_invocation_client()
            self.assertIsInstance(client, AbilityInvocationClient)
            result = client.invoke(
                AbilityCallRequest(
                    caller_ura="easynet:///r/example/agent/alice.sdk",
                    callee_ura="easynet:///r/example/device/dev-a",
                    subject_ura="easynet:///r/example/device/dev-a",
                    nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                    causal_context={"form": "none"},
                    ability_ura="easynet:///r/example/ability/device.dev-a.observe.health",
                    args={},
                )
            )
            env.close()

        self.assertTrue(result.ok)
        self.assertEqual(raw.init_paths, ["/tmp/control.json", "/tmp/control.json"])
        self.assertEqual(
            [entry[0] for entry in raw.identity_requests],
            ["build_descriptor_ref"],
        )
        self.assertEqual(raw.runtime_requests[0][0], "invoke")
        self.assertEqual(raw.shutdown_handles, [42, 42])

    def test_daemon_handle_profile_factories_use_attached_cabi_handle(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            handle = env.daemon_control().attach(
                AttachOptions(control_path="/tmp/control.json")
            )
            directory = handle.directory(ConnectOptions(max_message_bytes=4096))
            receipt = handle.receipts()
            admin = handle.admin()
            health = handle.health()

            self.assertIsInstance(directory, DirectoryClient)
            self.assertIsInstance(receipt, ReceiptClient)
            self.assertIsInstance(admin, AdminClient)
            self.assertIsInstance(health, HealthClient)
            self.assertTrue(health.runtime_health().ready())

            directory.close()
            receipt.close()
            admin.close()
            health.close()
            handle.detach()
            env.close()

        self.assertEqual(
            raw.daemon_attaches,
            [{"control_path": "/tmp/control.json"}],
        )
        self.assertEqual(raw.daemon_open_clients, [707, 707, 707, 707])
        self.assertEqual(raw.daemon_detaches, [707])
        self.assertEqual(raw.shutdown_handles, [808, 808, 808, 808])

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
            publication.list_abilities(
                PublishedAbilityQuery(
                    caller_ura=_CALLER,
                    callee_ura=_CALLEE,
                    subject_ura=_SUBJECT,
                    descriptor_version="1.0.0",
                    nonce_base64=_NONCE,
                    causal_context=_CAUSAL,
                )
            )
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
            mission.run_eal(MissionRunRequest(base=_mission_base(), source="mission weather"))
            admin.build_agent_list_invocation(AdminAgentListRequest(_admin_base()))
            admin.list_agents(AdminAgentListRequest(_admin_base()))
            admin.agent_start(
                AdminAgentStartRequest(
                    base=_admin_base(),
                    name="codex",
                    agent_type="codex",
                )
            )
            events.build_directory_subscription_invocation(
                EventsSubscriptionRequest(base=_events_base())
            )
            directory.build_list_devices_invocation(DeviceQuery(_directory_base()))
            surface.build_list_pages_invocation(
                SurfaceListPagesRequest(base=_surface_base())
            )
            surface.list_pages(SurfaceListPagesRequest(base=_surface_base()))
            surface.create_page(
                SurfaceCreatePageRequest(
                    base=_surface_base(),
                    project_id="docs",
                    folder="/tmp/easynet-pages-docs",
                    visibility="public",
                )
            )
            surface.surface_manifest(
                SurfaceManifestRequest(base=_surface_base(), project_id="docs")
            )
            compatibility.build_list_models_invocation(
                CompatibilityListModelsRequest(base=_compatibility_base())
            )
            compatibility.list_models(
                CompatibilityListModelsRequest(base=_compatibility_base())
            )
            compatibility.chat_completions(
                CompatibilityChatCompletionRequest(
                    base=_compatibility_base(),
                    request={
                        "model": "easynet:///r/example/ability/alice.codex.chat",
                        "messages": [
                            {"role": "user", "content": "reply with: ok"}
                        ],
                    },
                )
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
            compatibility.upload_file(
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
            compatibility.get_file(
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
            compatibility.delete_file(
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
            wrapper.transfer_file(
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
            wrapper.start_terminal_session(
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
            wrapper.start_remote_desktop_session(
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
            wrapper.start_browser_session(
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
            wrapper.start_media_session(
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
        self.assertIn("easynet_publication_build_list_abilities_invocation", symbols)
        self.assertIn("easynet_publication_project_ability_page", symbols)
        self.assertIn("easynet_host_binding_build", symbols)
        self.assertIn("easynet_mission_build_run_eal_invocation", symbols)
        self.assertIn("easynet_mission_project_status", symbols)
        self.assertIn("easynet_admin_build_agent_list_invocation", symbols)
        self.assertIn("easynet_admin_project_agent_records", symbols)
        self.assertIn("easynet_admin_build_agent_start_invocation", symbols)
        self.assertIn("easynet_admin_project_agent_lifecycle_result", symbols)
        self.assertIn("easynet_events_build_directory_subscription_invocation", symbols)
        self.assertIn("easynet_directory_build_list_devices_invocation", symbols)
        self.assertIn("easynet_surface_build_list_pages_invocation", symbols)
        self.assertIn("easynet_surface_project_page_page", symbols)
        self.assertIn("easynet_surface_build_create_page_invocation", symbols)
        self.assertIn("easynet_surface_project_page_record", symbols)
        self.assertIn("easynet_surface_build_manifest_invocation", symbols)
        self.assertIn("easynet_surface_project_manifest", symbols)
        self.assertIn("easynet_compatibility_build_list_models_invocation", symbols)
        self.assertIn("easynet_compatibility_project_model_page", symbols)
        self.assertIn("easynet_compatibility_build_chat_completion_invocation", symbols)
        self.assertIn("easynet_compatibility_project_chat_completion", symbols)
        self.assertIn("easynet_compatibility_build_file_upload_invocation", symbols)
        self.assertIn("easynet_compatibility_project_file_upload", symbols)
        self.assertIn("easynet_compatibility_build_file_retrieve_invocation", symbols)
        self.assertIn("easynet_compatibility_project_file", symbols)
        self.assertIn("easynet_compatibility_build_file_delete_invocation", symbols)
        self.assertIn("easynet_compatibility_project_file_delete_result", symbols)
        self.assertIn("easynet_wrappers_build_file_transfer_invocation", symbols)
        self.assertIn("easynet_wrappers_project_file_record", symbols)
        self.assertIn("easynet_wrappers_build_terminal_session_invocation", symbols)
        self.assertIn("easynet_wrappers_project_terminal_session", symbols)
        self.assertIn(
            "easynet_wrappers_build_remote_desktop_session_invocation", symbols
        )
        self.assertIn("easynet_wrappers_project_remote_desktop_session", symbols)
        self.assertIn("easynet_wrappers_build_browser_session_invocation", symbols)
        self.assertIn("easynet_wrappers_project_browser_session", symbols)
        self.assertIn("easynet_wrappers_build_media_session_invocation", symbols)
        self.assertIn("easynet_wrappers_project_media_session", symbols)
        runtime_abilities = [
            entry[1]["metadata"]["system_ability"]
            for entry in raw.runtime_requests
            if entry[0] == "invoke"
        ]
        self.assertIn("mission.run", runtime_abilities)
        self.assertIn("agent.list", runtime_abilities)
        self.assertIn("agent.start", runtime_abilities)
        self.assertIn("project_list", runtime_abilities)
        self.assertIn("pages.publish", runtime_abilities)
        self.assertIn("pages.get", runtime_abilities)
        self.assertIn("openai.list_models", runtime_abilities)
        self.assertIn("openai.chat_completions", runtime_abilities)
        self.assertIn("openai.files.upload", runtime_abilities)
        self.assertIn("openai.files.retrieve", runtime_abilities)
        self.assertIn("openai.files.delete", runtime_abilities)
        self.assertIn("wrapper.file.transfer", runtime_abilities)
        self.assertIn("wrapper.terminal.start", runtime_abilities)
        self.assertIn("wrapper.remote_desktop.start", runtime_abilities)
        self.assertIn("wrapper.browser.start", runtime_abilities)
        self.assertIn("wrapper.media.start", runtime_abilities)

    def test_publication_deploy_uses_cabi_carrier_and_runtime_core(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            publication = env.publication_client()
            resource_ref = publication.build_local_resource_ref(
                LocalResourceRefRequest(path="/tmp/package", capability="read")
            )
            result = publication.deploy_ability(
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

        self.assertEqual(result.state, "enabled")
        self.assertEqual(result.kind, "ability_deploy_result")
        self.assertIn(
            "easynet_publication_build_deploy_invocation",
            [item[0] for item in raw.profile_requests],
        )
        self.assertEqual(
            raw.profile_requests[-1][0],
            "easynet_publication_project_deploy_result",
        )
        self.assertEqual(raw.runtime_requests[-1][0], "invoke")
        self.assertEqual(
            raw.runtime_requests[-1][1]["metadata"]["system_ability"],
            "ability.deploy",
        )
        self.assertEqual(raw.shutdown_handles, [42, 42])

    def test_mission_run_uses_cabi_carrier_and_runtime_core(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            mission = env.mission_client()
            status = mission.run_eal(
                MissionRunRequest(base=_mission_base(), source="mission weather")
            )
            env.close()

        self.assertEqual(status.status.kind, "mission_status")
        self.assertEqual(status.status.mission_id, "mission-1")
        self.assertEqual(
            [item[0] for item in raw.profile_requests[-2:]],
            [
                "easynet_mission_build_run_eal_invocation",
                "easynet_mission_project_status",
            ],
        )
        self.assertEqual(raw.runtime_requests[-1][0], "invoke")
        self.assertEqual(
            raw.runtime_requests[-1][1]["metadata"]["system_ability"],
            "mission.run",
        )
        self.assertEqual(raw.shutdown_handles, [42, 42])

    def test_surface_list_uses_cabi_carrier_and_runtime_core(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            surface = env.surface_client()
            page = surface.list_pages(SurfaceListPagesRequest(base=_surface_base()))
            env.close()

        self.assertEqual(page.kind, "surface_page_page")
        self.assertEqual(
            [item[0] for item in raw.profile_requests[-2:]],
            [
                "easynet_surface_build_list_pages_invocation",
                "easynet_surface_project_page_page",
            ],
        )
        self.assertEqual(raw.runtime_requests[-1][0], "invoke")
        self.assertEqual(
            raw.runtime_requests[-1][1]["metadata"]["system_ability"],
            "project_list",
        )
        self.assertEqual(raw.shutdown_handles, [42, 42])

    def test_wrapper_transfer_uses_cabi_carrier_and_runtime_core(self) -> None:
        raw = FakeRawCABI()
        with _load_patch(raw):
            env = SdkEnvironment(control_path="/tmp/control.json")
            wrapper = env.wrapper_client()
            record = wrapper.transfer_file(
                WrapperFileTransferRequest(
                    base=_wrapper_base(),
                    file=WrapperFileRecordRequest(
                        file_ref="easynet:///r/example/resource/alice.files/report.txt",
                        owner_ura=_CALLER,
                        content_type="text/plain",
                        size_bytes=42,
                    ),
                )
            )
            env.close()

        self.assertEqual(record.kind, "file_record")
        self.assertEqual(
            record.file_ref,
            "easynet:///r/example/resource/alice.files/report.txt",
        )
        self.assertEqual(
            [item[0] for item in raw.profile_requests[-2:]],
            [
                "easynet_wrappers_build_file_transfer_invocation",
                "easynet_wrappers_project_file_record",
            ],
        )
        self.assertEqual(raw.runtime_requests[-1][0], "invoke")
        self.assertEqual(
            raw.runtime_requests[-1][1]["metadata"]["system_ability"],
            "wrapper.file.transfer",
        )
        self.assertEqual(raw.shutdown_handles, [42, 42])

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


def _write_control_discovery(tmp: str) -> Path:
    path = Path(tmp) / "control.json"
    path.write_text(
        json.dumps(
            {
                "socket_path": f"{tmp}/control.sock",
                "invocation_endpoint": f"{tmp}/daemon.sock",
                "pid": 123,
                "daemon_version": "1.2.3",
                "supported_ipc_versions": {"min": 1, "max": 1},
                "capability_flags": ["runtime"],
            }
        ),
        encoding="utf-8",
    )
    return path


if __name__ == "__main__":
    unittest.main()
