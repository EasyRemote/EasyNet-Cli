import inspect
import json
import pathlib
import unittest

from easynet_sdk import (
    AbilityQuery,
    AbilityDeployRequest,
    AbilityPackageManifest,
    AdminAgentListRequest,
    AdminAgentRefreshRequest,
    AdminAgentStartRequest,
    AdminAgentStopRequest,
    AdminCarrierBase,
    AdminClient,
    AdminGatewayStatusRequest,
    AdminSessionListRequest,
    AgentQuery,
    AttachOptions,
    CompatibilityCarrierBase,
    CompatibilityChatCompletionRequest,
    CompatibilityClient,
    CompatibilityFileDeleteRequest,
    CompatibilityFileRequest,
    CompatibilityFileUploadRequest,
    CompatibilityListModelsRequest,
    CompatibilityStreamChatCompletionRequest,
    Client,
    DaemonControl,
    DaemonHandle,
    DescriptorRefRequest,
    DeviceQuery,
    DirectoryClient,
    DirectoryQueryBase,
    ErrorCode,
    EventClient,
    EventCursor,
    EventDropReportInput,
    EventFrame,
    EventProjectionInput,
    EventTerminalInput,
    EventsCarrierBase,
    EventsDirectorySubscriptionRequest,
    GatewayStatus,
    HOST_STREAM_FRAME_SCHEMA,
    HOST_STREAM_HASH_ALGORITHM,
    HealthClient,
    HostBindingClient,
    HostStreamBindingRequest,
    HostStreamEnvelope,
    HostStreamEnvelopeRequest,
    HostStreamHashState,
    HostStreamTerminalSummary,
    IdentityClient,
    IdentityProjection,
    IdentityProjectionRequest,
    InvocationBuilder,
    InvocationDraft,
    InvocationSignature,
    LocalResourceRefRequest,
    MAX_DIRECTORY_PAGE_SIZE,
    MissionCancelRequest,
    MissionCarrierBase,
    MissionClient,
    MissionRunFileRequest,
    MissionRunRequest,
    MissionStatus,
    MissionTrackRequest,
    PreparedInvocation,
    SignedInvocation,
    PublicationClient,
    ReceiptClient,
    ReceiptFetchRequest,
    ReceiptSummary,
    ReceiptVerification,
    ResourceRef,
    ResolveQuery,
    RuntimeClient,
    RuntimeHealth,
    PrepareOptions,
    RetryHint,
    SDKError,
    MAX_SURFACE_PAGE_SIZE,
    BidiFrame,
    BidiSession,
    BidiState,
    StreamHandle,
    StreamState,
    SurfaceCarrierBase,
    SurfaceClient,
    SurfaceCreatePageRequest,
    SurfaceDeletePageRequest,
    SurfaceListPagesRequest,
    SurfaceManifest,
    SurfaceManifestRequest,
    SurfacePagePage,
    WrapperBrowserSessionRecord,
    WrapperFileRecord,
    WrapperFileRecordRequest,
    WrapperClient,
    WrapperMediaSessionRecord,
    WrapperRemoteDesktopSessionRecord,
    WrapperTerminalSessionRecord,
    WrapperTerminalSessionRequest,
    CausalRef,
    UnpublishAbilityRequest,
    ValidatePackageOptions,
    build_receipt_fetch_invocation,
)


ROOT = pathlib.Path(__file__).resolve().parents[3]
SHARED_CONFORMANCE_CASE_ROOT = "sdk/conformance/cases"
SHARED_CONFORMANCE_FIXTURE_ROOT = "sdk/conformance/fixtures"
CASES = ROOT / SHARED_CONFORMANCE_CASE_ROOT
FIXTURES = ROOT / SHARED_CONFORMANCE_FIXTURE_ROOT


def shared_case(name: str) -> str:
    return (CASES / name).read_text(encoding="utf-8")


def shared_fixture(name: str) -> bytes:
    return (FIXTURES / name).read_bytes()


class SharedHostBindingTransport:
    def __init__(self) -> None:
        self.binding_json = shared_fixture("host-stream-binding.v4.json")
        self.request_json = shared_fixture("host-stream-request.v4.json")
        self.item_json = shared_fixture("host-stream-frame.v4.json")
        self.terminal_json = self._terminal_frame_json()
        self.hash_json = shared_fixture("host-stream-hash-state.v4.json")

    def _terminal_frame_json(self) -> bytes:
        summary = json.loads(shared_fixture("host-stream-terminal.v4.json"))
        return json.dumps(
            {
                "frame_type": "terminal",
                "seq": summary["frames"],
                "value": None,
                "error": None,
                "terminal": summary,
                "output_hash": summary["output_hash"],
            },
            separators=(",", ":"),
        ).encode("utf-8")

    def build_host_stream_binding(self, request_json: bytes) -> bytes:
        return self.binding_json

    def decode_request(self, envelope_json: bytes) -> bytes:
        return self.request_json

    def encode_item(self, request_json: bytes) -> bytes:
        return self.item_json

    def encode_error(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared conformance fixture test",
        )

    def encode_terminal(self, request_json: bytes) -> bytes:
        return self.terminal_json

    def fold_output_hash(self, request_json: bytes) -> bytes:
        return self.hash_json

    def close(self) -> None:
        return None


class SharedDirectoryTransport:
    def __init__(self) -> None:
        self.expected_devices_request = shared_fixture(
            "directory-list-devices-request.v4.json"
        )
        self.expected_agents_request = shared_fixture(
            "directory-list-agents-request.v4.json"
        )
        self.expected_ability_request = shared_fixture(
            "directory-list-abilities-request.v4.json"
        )
        self.expected_resolve_request = shared_fixture("directory-resolve-request.v4.json")
        self.devices_json = shared_fixture("directory-device-page.v4.json")
        self.agents_json = shared_fixture("directory-agent-page.v4.json")
        self.abilities_json = shared_fixture("directory-ability-page.v4.json")
        self.resolve_json = shared_fixture("directory-resolved-ref.v4.json")

    def build_directory_subscription_invocation(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared directory conformance fixture test",
        )

    def resolve(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_resolve_request)
        return self.resolve_json

    def list_devices(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_devices_request)
        return self.devices_json

    def list_agents(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_agents_request)
        return self.agents_json

    def list_abilities(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_ability_request)
        return self.abilities_json

    def subscribe_directory(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared directory conformance fixture test",
        )

    def close(self) -> None:
        return None


class SharedIdentityTransport:
    def __init__(self) -> None:
        self.descriptor_json = shared_fixture("identity.descriptor-ref.v4.json")
        self.ability_json = (
            b'{"kind":"ability","valid":true,'
            b'"ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
            b'"realm":"example","profile":"easynet-strict-v2",'
            b'"display_id":"device.dev-a.observe.health",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a",'
            b'"owner_kind":"device","public_name":"observe.health",'
            b'"local_registry_ability":"observe.health"},'
            b'"metadata":{"grammar_owner":"axon","source":"shared-conformance"}}'
        )

    def project_descriptor_ref(self, request_json: bytes) -> bytes:
        request = json.loads(request_json.decode("utf-8"))
        if (
            request.get("descriptor_ref")
            != "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
        ):
            raise AssertionError(f"unexpected descriptor projection request: {request}")
        return self.descriptor_json

    def build_descriptor_ref(self, request_json: bytes) -> bytes:
        request = json.loads(request_json.decode("utf-8"))
        if request != {
            "ability_ura": "easynet:///r/example/ability/device.dev-a.observe.health",
            "descriptor_version": "1.0.0",
        }:
            raise AssertionError(f"unexpected descriptor build request: {request}")
        return self.descriptor_json

    def project_identity(self, request_json: bytes) -> bytes:
        request = json.loads(request_json.decode("utf-8"))
        if request.get("ura") != "easynet:///r/example/ability/device.dev-a.observe.health":
            raise AssertionError(f"unexpected identity projection request: {request}")
        return self.ability_json

    def build_ura(self, request_json: bytes) -> bytes:
        request = json.loads(request_json.decode("utf-8"))
        if request != {
            "ability_name": "observe.health",
            "kind": "ability",
            "owner_ura": "easynet:///r/example/device/dev-a",
        }:
            raise AssertionError(f"unexpected identity build request: {request}")
        return self.ability_json

    def build_resource_ref(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared identity conformance fixture test",
        )

    def register_signing_key(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared identity conformance fixture test",
        )

    def list_signing_keys(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared identity conformance fixture test",
        )

    def revoke_signing_key(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared identity conformance fixture test",
        )

    def signer(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared identity conformance fixture test",
        )

    def close(self) -> None:
        return None


class SharedPublicationTransport:
    def __init__(self) -> None:
        self.expected_resource_request = shared_fixture("local-resource-ref-request.v4.json")
        self.expected_validate_request = shared_publication_validate_package_request()
        self.expected_deploy_request = shared_fixture("ability-deploy-request.v4.json")
        self.resource_json = shared_fixture("resource-ref.local-fs.v4.json")
        self.validation_json = shared_fixture("package-validation.v4.json")
        self.deploy_invocation_json = shared_fixture("publication-deploy-invocation.v4.json")
        self.unpublish_invocation_json = shared_fixture(
            "publication-unpublish-invocation.v4.json"
        )

    def build_resource_ref(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_resource_request)
        return self.resource_json

    def validate_package(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_validate_request)
        return self.validation_json

    def deploy_ability(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared publication conformance fixture test",
        )

    def build_deploy_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_deploy_request)
        return self.deploy_invocation_json

    def install_plugin(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared publication conformance fixture test",
        )

    def list_abilities(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared publication conformance fixture test",
        )

    def show_ability(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared publication conformance fixture test",
        )

    def enable_ability_impl(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared publication conformance fixture test",
        )

    def disable_ability_impl(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared publication conformance fixture test",
        )

    def build_unpublish_invocation(self, request_json: bytes) -> bytes:
        request = json.loads(request_json.decode("utf-8"))
        if request.get("ability_ura") != "easynet:///r/example/ability/device.dev-a.er.weather":
            raise AssertionError(f"unexpected unpublish request: {request}")
        if request.get("caller_ura") != "easynet:///r/example/agent/alice.sdk":
            raise AssertionError(f"unexpected unpublish caller: {request}")
        return self.unpublish_invocation_json

    def unpublish_ability(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared publication conformance fixture test",
        )

    def close(self) -> None:
        return None


class SharedMissionTransport:
    def __init__(self) -> None:
        self.expected_run_request = shared_fixture("mission-run-request.v4.json")
        self.expected_run_file_request = shared_fixture("mission-run-file-request.v4.json")
        self.expected_track_request = shared_fixture("mission-track-request.v4.json")
        self.expected_cancel_request = shared_fixture("mission-cancel-request.v4.json")
        self.run_invocation_json = shared_fixture("mission-run-invocation.v4.json")
        self.track_invocation_json = shared_fixture("mission-track-invocation.v4.json")
        self.cancel_invocation_json = shared_fixture("mission-cancel-invocation.v4.json")
        self.status_json = shared_fixture("mission-status.v4.json")

    def build_run_eal_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_run_request)
        return self.run_invocation_json

    def build_run_file_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_run_file_request)
        return self.run_invocation_json

    def build_track_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_track_request)
        return self.track_invocation_json

    def build_cancel_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_cancel_request)
        return self.cancel_invocation_json

    def run_eal(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared mission conformance fixture test",
        )

    def run_file(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared mission conformance fixture test",
        )

    def track(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_track_request)
        return self.status_json

    def cancel(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_cancel_request)
        return self.status_json

    def events(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared mission conformance fixture test",
        )

    def close(self) -> None:
        return None


class SharedAdminGatewayTransport:
    def __init__(self) -> None:
        self.expected_agent_list_request = shared_fixture(
            "admin-agent-list-request.v4.json"
        )
        self.expected_agent_start_request = shared_fixture(
            "admin-agent-start-request.v4.json"
        )
        self.expected_agent_stop_request = shared_fixture(
            "admin-agent-stop-request.v4.json"
        )
        self.expected_agent_refresh_request = shared_fixture(
            "admin-agent-refresh-request.v4.json"
        )
        self.expected_session_list_request = shared_fixture(
            "admin-session-list-request.v4.json"
        )
        self.agent_list_invocation_json = shared_fixture(
            "admin-agent-list-invocation.v4.json"
        )
        self.agent_start_invocation_json = shared_fixture(
            "admin-agent-start-invocation.v4.json"
        )
        self.agent_stop_invocation_json = shared_fixture(
            "admin-agent-stop-invocation.v4.json"
        )
        self.agent_refresh_invocation_json = shared_fixture(
            "admin-agent-refresh-invocation.v4.json"
        )
        self.session_list_invocation_json = shared_fixture(
            "admin-session-list-invocation.v4.json"
        )
        self.gateway_status_json = shared_fixture("gateway-status.v4.json")
        self.agent_records_json = shared_fixture("admin-agent-records.v4.json")
        self.agent_lifecycle_result_json = shared_fixture(
            "admin-agent-lifecycle-result.v4.json"
        )

    def build_agent_list_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_agent_list_request)
        return self.agent_list_invocation_json

    def build_agent_start_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_agent_start_request)
        return self.agent_start_invocation_json

    def build_agent_stop_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_agent_stop_request)
        return self.agent_stop_invocation_json

    def build_agent_refresh_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_agent_refresh_request)
        return self.agent_refresh_invocation_json

    def build_session_list_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_session_list_request)
        return self.session_list_invocation_json

    def gateway_status(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, b"{}")
        return self.gateway_status_json

    def list_agents(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_agent_list_request)
        return self.agent_records_json

    def agent_start(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_agent_start_request)
        return self.agent_lifecycle_result_json

    def agent_stop(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_agent_stop_request)
        return self.agent_lifecycle_result_json

    def agent_refresh(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_agent_refresh_request)
        return self.agent_lifecycle_result_json

    def list_device_sessions(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared admin gateway conformance fixture test",
        )

    def join_hub(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared admin gateway conformance fixture test",
        )

    def leave_hub(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared admin gateway conformance fixture test",
        )

    def pairing_preflight(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared admin gateway conformance fixture test",
        )

    def validate_pairing(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared admin gateway conformance fixture test",
        )

    def verify_device_credential(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared admin gateway conformance fixture test",
        )

    def create_pairing(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared admin gateway conformance fixture test",
        )

    def revoke_device(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared admin gateway conformance fixture test",
        )

    def create_device_session(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared admin gateway conformance fixture test",
        )

    def delete_device_session(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared admin gateway conformance fixture test",
        )

    def close(self) -> None:
        return None


class SharedEventsTransport:
    def __init__(self) -> None:
        self.expected_directory_subscription_request = (
            shared_events_directory_subscription_request_json()
        )
        self.expected_directory_projection_input = shared_events_projection_input_json()
        self.expected_drop_report_input = shared_events_drop_report_input_json()
        self.expected_terminal_input = shared_events_terminal_input_json()
        self.directory_subscription_invocation_json = shared_fixture(
            "events-directory-subscription-invocation.v4.json"
        )
        self.directory_event_json = shared_fixture("event.directory.v4.json")
        self.drop_report_json = shared_fixture("event.directory-drop-report.v4.json")
        self.terminal_json = shared_fixture("event.directory-terminal.v4.json")

    def build_directory_subscription_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(
            request_json, self.expected_directory_subscription_request
        )
        return self.directory_subscription_invocation_json

    def build_device_subscription_invocation(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared events conformance fixture test",
        )

    def build_session_subscription_invocation(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared events conformance fixture test",
        )

    def build_invocation_subscription_invocation(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared events conformance fixture test",
        )

    def subscribe_directory(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared events conformance fixture test",
        )

    def subscribe_devices(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared events conformance fixture test",
        )

    def subscribe_sessions(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared events conformance fixture test",
        )

    def subscribe_invocations(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared events conformance fixture test",
        )

    def list_device_events(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared events conformance fixture test",
        )

    def project_directory_event(self, event_json: bytes) -> bytes:
        assert_json_equivalent(event_json, self.expected_directory_projection_input)
        return self.directory_event_json

    def project_drop_report(self, drop_json: bytes) -> bytes:
        assert_json_equivalent(drop_json, self.expected_drop_report_input)
        return self.drop_report_json

    def project_terminal(self, terminal_json: bytes) -> bytes:
        assert_json_equivalent(terminal_json, self.expected_terminal_input)
        return self.terminal_json

    def close(self) -> None:
        return None


class SharedSurfaceTransport:
    def __init__(self) -> None:
        self.expected_list_request = shared_fixture("surface-list-pages-request.v4.json")
        self.expected_create_request = shared_fixture("surface-create-page-request.v4.json")
        self.expected_delete_request = shared_fixture("surface-delete-page-request.v4.json")
        self.expected_manifest_request = shared_fixture("surface-manifest-request.v4.json")
        self.list_invocation_json = shared_fixture("surface-list-pages-invocation.v4.json")
        self.create_invocation_json = shared_fixture(
            "surface-create-page-invocation.v4.json"
        )
        self.delete_invocation_json = shared_fixture(
            "surface-delete-page-invocation.v4.json"
        )
        self.manifest_invocation_json = shared_fixture(
            "surface-manifest-invocation.v4.json"
        )
        self.page_page_json = shared_fixture("surface-page-page.v4.json")
        self.manifest_json = shared_fixture("surface-manifest.v4.json")

    def build_list_pages_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_list_request)
        return self.list_invocation_json

    def build_create_page_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_create_request)
        return self.create_invocation_json

    def build_delete_page_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_delete_request)
        return self.delete_invocation_json

    def build_manifest_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_manifest_request)
        return self.manifest_invocation_json

    def build_health_invocation(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared surface conformance fixture test",
        )

    def list_pages(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_list_request)
        return self.page_page_json

    def create_page(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared surface conformance fixture test",
        )

    def delete_page(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared surface conformance fixture test",
        )

    def surface_manifest(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_manifest_request)
        return self.manifest_json

    def public_page_ref(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared surface conformance fixture test",
        )

    def surface_health(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared surface conformance fixture test",
        )

    def close(self) -> None:
        return None


class SharedCompatibilityTransport:
    def __init__(self) -> None:
        self.expected_list_request = shared_fixture(
            "compatibility-list-models-request.v4.json"
        )
        self.expected_chat_request = shared_fixture(
            "compatibility-chat-completion-request.v4.json"
        )
        self.list_invocation_json = shared_fixture(
            "compatibility-list-models-invocation.v4.json"
        )
        self.chat_invocation_json = shared_fixture(
            "compatibility-chat-completion-invocation.v4.json"
        )
        self.stream_invocation_json = shared_fixture(
            "compatibility-stream-chat-completion-invocation.v4.json"
        )
        self.model_page_json = shared_fixture("compatibility-model-page.v4.json")
        self.chat_completion_json = shared_fixture("compatibility-chat-completion.v4.json")
        self.chat_stream_json = shared_fixture("compatibility-chat-stream.v4.json")

    def build_list_models_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_list_request)
        return self.list_invocation_json

    def build_chat_completion_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_chat_request)
        return self.chat_invocation_json

    def build_stream_chat_completion_invocation(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, shared_compatibility_stream_request_json())
        return self.stream_invocation_json

    def build_file_upload_invocation(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared compatibility conformance fixture test",
        )

    def build_file_retrieve_invocation(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared compatibility conformance fixture test",
        )

    def build_file_delete_invocation(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared compatibility conformance fixture test",
        )

    def list_models(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_list_request)
        return self.model_page_json

    def create_chat_completion(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, self.expected_chat_request)
        return self.chat_completion_json

    def stream_chat_completion(self, request_json: bytes) -> bytes:
        assert_json_equivalent(request_json, shared_compatibility_stream_request_json())
        return self.chat_stream_json

    def upload_file(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared compatibility conformance fixture test",
        )

    def retrieve_file(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared compatibility conformance fixture test",
        )

    def delete_file(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared compatibility conformance fixture test",
        )

    def close(self) -> None:
        return None


class SharedDiscoveryTransport:
    def __init__(self, abi_version: int) -> None:
        self.abi_version = abi_version

    def feature_discovery(self) -> bytes:
        return shared_feature_discovery_json(self.abi_version)

    def close(self) -> None:
        return None


class SharedHealthTransport:
    def __init__(self, payload: bytes) -> None:
        self.payload = payload

    def runtime_health(self) -> bytes:
        return self.payload


class SharedControlOnlyDaemonTransport:
    def discover(self, options_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared runtime core conformance fixture test",
        )

    def start(self, config_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared runtime core conformance fixture test",
        )

    def attach(self, options_json: bytes) -> bytes:
        return (
            b'{"handle_id":"daemon-control-only","state":"ControlOnly","mode":"hub",'
            b'"endpoints":{"control_endpoint":"unix:///tmp/easynet-control.sock"},'
            b'"diagnostics":["invocation endpoint unavailable"]}'
        )

    def status(self, handle_id: str) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared runtime core conformance fixture test",
        )

    def open_runtime(self, handle_id: str, options_json: bytes) -> tuple[object, bytes]:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared runtime core conformance fixture test",
        )

    def stop(self, handle_id: str, options_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared runtime core conformance fixture test",
        )

    def detach(self, handle_id: str) -> None:
        return None


class SharedBidiLifecycleTransport:
    def __init__(self) -> None:
        self.recv_frames = [
            b'{"sequence":1,"kind":"data","stream_id":1}',
            b'{"sequence":2,"kind":"remote_close_send","stream_id":1}',
        ]
        self.closed = False

    def send(self, frame_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared stream-bidi lifecycle conformance test",
        )

    def recv(self) -> bytes:
        if not self.recv_frames:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="test",
                retry=RetryHint.NEVER,
                message="no shared bidi lifecycle frame",
            )
        return self.recv_frames.pop(0)

    def close_send(self) -> bytes:
        return b'{"session_id":"bidi-lifecycle-1","state":"HalfClosedLocal","terminal":false}'

    def close(self) -> None:
        self.closed = True

    def cancel(self, reason: str) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared stream-bidi lifecycle conformance test",
        )


class SharedConformanceFixtureTests(unittest.TestCase):
    def test_python_facade_executes_shared_runtime_core_conformance_cases(self) -> None:
        complete_tuple_case = shared_case("invocation-complete-tuple.yaml")
        self._require_case_id(complete_tuple_case, "invocation/complete_tuple")
        self._require_case_action(complete_tuple_case, "build_invocation")
        self._require_case_action(complete_tuple_case, "remove_field")
        self._require_case_action(complete_tuple_case, "prepare")
        self._require_case_fixture(complete_tuple_case, "invocation.complete.v4.json")
        self._require_case_expectation(complete_tuple_case, "error_code: InvalidArgument")

        draft = InvocationDraft.from_json(shared_fixture("invocation.complete.v4.json"))
        self.assertEqual(draft.caller_ura, "easynet:///r/example/agent/alice.sdk")
        self.assertIn("args", json.loads(draft.to_json()))

        missing_caller = json.loads(shared_fixture("invocation.complete.v4.json"))
        del missing_caller["caller_ura"]
        with self.assertRaises(SDKError) as caught:
            InvocationDraft.from_json(json.dumps(missing_caller))
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        prepared = PreparedInvocation.from_json(
            shared_fixture("prepared.signing-material.v4.json")
        )
        self.assertFalse(prepared.submit_ready())
        self.assertEqual(prepared.signing_material.algorithm, "ed25519")

        runtime_error = SDKError.from_json(shared_fixture("runtime.error.v4.json"))
        assert runtime_error is not None
        self.assertEqual(runtime_error.code, ErrorCode.INVALID_ARGUMENT)
        self.assertEqual(runtime_error.retry, RetryHint.NEVER)

        health_case = shared_case("health-api-vs-runtime.yaml")
        self._require_case_id(health_case, "health/api_vs_runtime")
        self._require_case_action(health_case, "read_health")
        self._require_case_fixture(health_case, "health.ready.v4.json")
        self._require_case_expectation(health_case, "api_ready_field: api_ready")
        self._require_case_expectation(health_case, "runtime_ready_field: runtime_ready")

        health = RuntimeHealth.from_json(shared_fixture("health.ready.v4.json"))
        self.assertTrue(health.api_alive())
        self.assertTrue(health.ready())

    def test_python_runtime_core_executes_shared_lifecycle_version_error_conformance_cases(self) -> None:
        compatible_case = shared_case("version-abi-compatible.yaml")
        self._require_case_id(compatible_case, "version/abi_compatible")
        self._require_case_action(compatible_case, "feature_discovery")
        self._require_case_expectation(compatible_case, "result: ok")
        self._require_case_expectation(compatible_case, "abi_version: 4")

        compatible = Client(SharedDiscoveryTransport(4))
        features = compatible.require_abi(4)
        self.assertEqual(features.version().abi_version, 4)

        incompatible_case = shared_case("version-abi-incompatible.yaml")
        self._require_case_id(incompatible_case, "version/abi_incompatible")
        self._require_case_action(incompatible_case, "feature_discovery")
        self._require_case_expectation(incompatible_case, "result: error")
        self._require_case_expectation(
            incompatible_case, "error_code: VersionIncompatible"
        )

        incompatible = Client(SharedDiscoveryTransport(0))
        with self.assertRaises(SDKError) as caught:
            incompatible.require_abi(4)
        self.assertEqual(caught.exception.code, ErrorCode.VERSION_INCOMPATIBLE)

        control_only_case = shared_case("daemon-control-only.yaml")
        self._require_case_id(control_only_case, "daemon/control_only")
        self._require_case_action(control_only_case, "attach_daemon")
        self._require_case_fixture(control_only_case, "health.ready.v4.json")
        self._require_case_expectation(control_only_case, "error_code: ControlOnly")

        health_client = HealthClient(
            SharedHealthTransport(shared_control_only_health_json())
        )
        health = health_client.runtime_health()
        self.assertTrue(health.api_alive())
        self.assertFalse(health.ready())
        self.assertFalse(health.invocation_ready)

        control = DaemonControl(SharedControlOnlyDaemonTransport())
        with self.assertRaises(SDKError) as caught:
            control.attach(AttachOptions())
        self.assertEqual(caught.exception.code, ErrorCode.CONTROL_ONLY)

        error_case = shared_case("error-typed-json.yaml")
        self._require_case_id(error_case, "error/typed_json")
        self._require_case_action(error_case, "trigger_invalid_handle_error")
        self._require_case_action(error_case, "read_last_error_json")
        self._require_case_action(error_case, "project_explicit_error_code")
        self._require_case_expectation(error_case, "schema: error.schema.json")
        self._require_case_expectation(error_case, "invalid_handle_code: INVALID_HANDLE")
        self._require_case_expectation(error_case, "explicit_timeout_code: TIMEOUT")
        self._require_case_expectation(
            error_case, "human_message_parse_required: false"
        )

        invalid_handle = SDKError.from_json(
            b'{"code":"INVALID_HANDLE","stage":"sdk","message":"invalid handle",'
            b'"retry":"never","source":"sdk","details":{}}'
        )
        self.assertIsNotNone(invalid_handle)
        assert invalid_handle is not None
        self.assertEqual(invalid_handle.code, ErrorCode.INVALID_HANDLE)
        self.assertEqual(invalid_handle.stage, "sdk")
        self.assertEqual(invalid_handle.retry, RetryHint.NEVER)

        timeout = SDKError.from_json(
            b'{"code":"TIMEOUT","stage":"invoke","message":"deadline exceeded",'
            b'"retry":"safe","source":"daemon","details":{}}'
        )
        self.assertIsNotNone(timeout)
        assert timeout is not None
        self.assertEqual(timeout.code, ErrorCode.TIMEOUT)
        self.assertEqual(timeout.retry, RetryHint.SAFE)
        self.assertTrue(timeout.retryable)

    def test_python_runtime_core_executes_shared_invocation_signing_conformance_cases(self) -> None:
        builder_case = shared_case("invocation-builder-handle-state.yaml")
        self._require_case_id(builder_case, "invocation/builder_handle_state")
        for action in (
            "create_builder",
            "set_complete_tuple",
            "inspect_builder",
            "build_builder",
        ):
            self._require_case_action(builder_case, action)
        self._require_case_expectation(builder_case, "result: error_after_build")
        self._require_case_expectation(builder_case, "build_consumes_handle: true")
        self._require_case_expectation(builder_case, "error_code: InvalidHandle")

        builder = shared_invocation_builder()
        builder.inspect()
        builder.build()
        with self.assertRaises(SDKError) as builder_caught:
            builder.inspect()
        self.assertEqual(builder_caught.exception.code, ErrorCode.INVALID_HANDLE)

        canonical_case = shared_case("invocation-canonical-material.yaml")
        self._require_case_id(canonical_case, "invocation/canonical_material")
        self._require_case_action(canonical_case, "prepare")
        self._require_case_fixture(canonical_case, "invocation.complete.v4.json")
        self._require_case_expectation(canonical_case, "material_owner: axon_delegated")
        self._require_case_expectation(
            canonical_case, "fixture: prepared.signing-material.v4.json"
        )

        class SharedInvocationSigningTransport:
            def __init__(self) -> None:
                self.seen_draft: bytes | None = None
                self.seen_signed: dict[str, object] | None = None

            def invoke(self, draft_json: bytes) -> bytes:
                raise SDKError(
                    code=ErrorCode.NOT_IMPLEMENTED,
                    stage="test",
                    retry=RetryHint.NEVER,
                    message="not used by shared invocation signing conformance test",
                )

            def open_stream(self, draft_json: bytes):
                raise SDKError(
                    code=ErrorCode.NOT_IMPLEMENTED,
                    stage="test",
                    retry=RetryHint.NEVER,
                    message="not used by shared invocation signing conformance test",
                )

            def open_bidi(self, draft_json: bytes, streams_json: bytes):
                raise SDKError(
                    code=ErrorCode.NOT_IMPLEMENTED,
                    stage="test",
                    retry=RetryHint.NEVER,
                    message="not used by shared invocation signing conformance test",
                )

            def prepare(self, draft_json: bytes, options_json: bytes) -> bytes:
                self.seen_draft = draft_json
                return shared_fixture("prepared.signing-material.v4.json")

            def submit_signed(self, signed_json: bytes) -> bytes:
                self.seen_signed = json.loads(signed_json.decode("utf-8"))
                return (
                    b'{"handle_id":7,"state":"Submitted","terminal":false,'
                    b'"events":[{"sequence":1,"kind":"submitted",'
                    b'"state":"Submitted","terminal":false}],"result":null}'
                )

            def await_handle(self, handle_id: int) -> bytes:
                return json.dumps(
                    {
                        "ok": True,
                        "tuple": json.loads(
                            shared_fixture("invocation.complete.v4.json")
                        ),
                        "terminal_state": "Completed",
                        "output_content_type": "application/json",
                        "output_base64": "e30=",
                        "output_json": {},
                        "elapsed_ms": 1,
                        "receipt": {"receipt_id": "receipt-1"},
                        "error": None,
                    },
                    separators=(",", ":"),
                    sort_keys=True,
                ).encode("utf-8")

            def cancel_handle(self, handle_id: int, reason: str) -> bytes:
                return (
                    b'{"handle_id":7,"cancelled":false,'
                    b'"state":"Completed","terminal":true}'
                )

            def handle_events(self, handle_id: int) -> bytes:
                return (
                    b'{"handle_id":7,"state":"Completed","terminal":true,'
                    b'"events":[{"sequence":1,"kind":"completed",'
                    b'"state":"Completed","terminal":true,'
                    b'"result":{"receipt_id":"receipt-1"}}],'
                    b'"result":{"receipt_id":"receipt-1"}}'
                )

            def free_handle(self, handle_id: int) -> None:
                return None

            def close(self) -> None:
                return None

        transport = SharedInvocationSigningTransport()
        client = RuntimeClient(transport)
        prepared, material = client.prepare(shared_invocation_draft(), PrepareOptions())
        assert transport.seen_draft is not None
        assert_json_equivalent(
            transport.seen_draft, shared_fixture("invocation.complete.v4.json")
        )
        self.assertEqual(material.algorithm, "ed25519")
        self.assertEqual(
            material.canonical_bytes_base64,
            "ZXhhbXBsZS1jYW5vbmljYWwtYnl0ZXM=",
        )
        self.assertFalse(prepared.submit_ready())

        not_submittable_case = shared_case("invocation-prepared-not-submittable.yaml")
        self._require_case_id(not_submittable_case, "invocation/prepared_not_submittable")
        self._require_case_action(not_submittable_case, "submit_prepared")
        self._require_case_expectation(not_submittable_case, "error_code: InvalidArgument")
        self.assertFalse(prepared.submit_ready())

        presigned_case = shared_case("invocation-presigned-submit.yaml")
        self._require_case_id(presigned_case, "invocation/presigned_submit")
        self._require_case_action(presigned_case, "attach_signature")
        self._require_case_action(presigned_case, "submit_signed")
        self._require_case_expectation(presigned_case, "signature_preserved: true")

        signed = prepared.sign_with_caller_signature(shared_invocation_signature())
        handle = client.submit_signed(signed)
        self.assertEqual(handle.handle_id, 7)
        assert transport.seen_signed is not None
        self.assertEqual(
            transport.seen_signed["signature"]["signature_base64"],
            "c2lnbmF0dXJl",
        )

        local_signing_case = shared_case("invocation-local-daemon-signing-boundary.yaml")
        self._require_case_id(
            local_signing_case, "invocation/local_daemon_signing_boundary"
        )
        self._require_case_action(local_signing_case, "local_daemon_sign")
        self._require_case_expectation(local_signing_case, "public_object: SignedInvocation")
        self.assertTrue(signed.submit_ready())
        self.assertFalse(signed.prepared.submit_ready())

        terminal_case = shared_case("invocation-handle-terminal-monotonicity.yaml")
        self._require_case_id(terminal_case, "invocation/handle_terminal_monotonicity")
        for action in (
            "prepare_complete_tuple",
            "sign_prepared",
            "submit_signed_handle",
            "await_handle_terminal",
            "cancel_handle",
            "read_handle_events",
        ):
            self._require_case_action(terminal_case, action)
        self._require_case_expectation(terminal_case, "submit_consumes_signed: true")
        self._require_case_expectation(terminal_case, "terminal_event_count: 1")

        result = client.await_result(handle)
        cancel = client.cancel(handle, "after terminal")
        events = client.events(handle)
        self.assertTrue(result.ok)
        self.assertEqual(result.terminal_state, "Completed")
        self.assertEqual(cancel.state, "Completed")
        self.assertTrue(cancel.terminal)
        self.assertTrue(events.terminal)
        self.assertEqual(len(events.events), 1)
        self.assertTrue(events.events[0].terminal)

    def test_python_runtime_core_executes_shared_stream_bidi_lifecycle_conformance_case(self) -> None:
        lifecycle_case = shared_case("stream-bidi-lifecycle-state.yaml")
        self._require_case_id(lifecycle_case, "stream_bidi/lifecycle_state")
        for action in (
            "open_stream",
            "close_stream",
            "open_bidi",
            "close_bidi_send",
            "send_bidi_after_close_send",
            "close_bidi",
        ):
            self._require_case_action(lifecycle_case, action)
        self._require_case_fixture(lifecycle_case, "invocation.complete.v4.json")
        for expectation in (
            "stream_close_unknown_is_idempotent: true",
            "stream_cross_owner_close_error: ERR_INVALID_HANDLE",
            "bidi_close_send_keeps_session_registered: true",
            "bidi_close_send_unknown_error: ERR_INVALID_HANDLE",
            "bidi_send_after_close_send_error: ERR_CANCELLED",
            "bidi_close_releases_session: true",
        ):
            self._require_case_expectation(lifecycle_case, expectation)

        InvocationDraft.from_json(shared_fixture("invocation.complete.v4.json"))

        class StreamCloseTransport:
            def __init__(self) -> None:
                self.close_calls = 0

            def recv(self) -> bytes:
                raise SDKError(
                    code=ErrorCode.NOT_IMPLEMENTED,
                    stage="test",
                    retry=RetryHint.NEVER,
                    message="not used by shared stream-bidi lifecycle conformance test",
                )

            def cancel(self, reason: str) -> bytes:
                raise SDKError(
                    code=ErrorCode.NOT_IMPLEMENTED,
                    stage="test",
                    retry=RetryHint.NEVER,
                    message="not used by shared stream-bidi lifecycle conformance test",
                )

            def close(self) -> None:
                self.close_calls += 1

        stream_transport = StreamCloseTransport()
        stream = StreamHandle.from_json(
            stream_transport,
            b'{"stream_id":"stream-lifecycle-1","state":"Open","max_buffered_events":4}',
        )
        stream.close()
        stream.close()
        self.assertEqual(stream.state, StreamState.CLOSED)
        self.assertEqual(stream_transport.close_calls, 1)

        class CrossOwnerStreamTransport(StreamCloseTransport):
            def close(self) -> None:
                raise SDKError(
                    code=ErrorCode.INVALID_HANDLE,
                    stage="stream",
                    retry=RetryHint.NEVER,
                    message="stream handle is not owned by caller",
                )

        cross_owner_stream = StreamHandle.from_json(
            CrossOwnerStreamTransport(),
            b'{"stream_id":"stream-cross-owner","state":"Open","max_buffered_events":4}',
        )
        with self.assertRaises(SDKError) as caught:
            cross_owner_stream.close()
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_HANDLE)

        bidi_transport = SharedBidiLifecycleTransport()
        bidi = BidiSession.from_json(
            bidi_transport,
            b'{"session_id":"bidi-lifecycle-1","state":"Open","max_buffered_frames":4}',
        )
        outcome = bidi.close_send()
        self.assertEqual(outcome.state, BidiState.HALF_CLOSED_LOCAL)
        self.assertFalse(outcome.terminal)
        self.assertEqual(bidi.state, BidiState.HALF_CLOSED_LOCAL)

        received = bidi.receive()
        self.assertEqual(received.kind, "data")
        self.assertEqual(bidi.state, BidiState.HALF_CLOSED_LOCAL)

        with self.assertRaises(SDKError) as caught:
            bidi.send(BidiFrame(sequence=1, kind="data", stream_id=1))
        self.assertEqual(caught.exception.code, ErrorCode.CANCELLED)
        self.assertEqual(bidi.state, BidiState.HALF_CLOSED_LOCAL)

        remote_close = bidi.receive()
        self.assertEqual(remote_close.kind, "remote_close_send")
        self.assertEqual(bidi.state, BidiState.TERMINAL)
        bidi.close()
        self.assertEqual(bidi.state, BidiState.CLOSED)
        self.assertTrue(bidi_transport.closed)

        class UnknownBidiCloseSendTransport(SharedBidiLifecycleTransport):
            def close_send(self) -> bytes:
                raise SDKError(
                    code=ErrorCode.INVALID_HANDLE,
                    stage="bidi",
                    retry=RetryHint.NEVER,
                    message="bidi session is not owned by caller",
                )

        unknown_bidi = BidiSession.from_json(
            UnknownBidiCloseSendTransport(),
            b'{"session_id":"bidi-cross-owner","state":"Open","max_buffered_frames":4}',
        )
        with self.assertRaises(SDKError) as caught:
            unknown_bidi.close_send()
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_HANDLE)

    def test_python_host_binding_executes_shared_conformance_case(self) -> None:
        host_binding_case = shared_case("host-binding-codec-hash.yaml")
        self._require_case_id(host_binding_case, "host_binding/codec_hash")
        for action in (
            "build_host_stream_binding",
            "decode_request",
            "encode_item",
            "fold_output_hash",
            "encode_terminal",
        ):
            self._require_case_action(host_binding_case, action)
        for fixture in (
            "host-stream-binding.v4.json",
            "host-stream-request.v4.json",
            "host-stream-frame.v4.json",
            "host-stream-terminal.v4.json",
            "host-stream-hash-state.v4.json",
        ):
            self._require_case_fixture(host_binding_case, fixture)
        self._require_case_expectation(
            host_binding_case, """canonical_json: '{"token":"hello"}'"""
        )
        self._require_case_expectation(
            host_binding_case, "rejects_hash_gap_or_reorder: true"
        )

        client = HostBindingClient(SharedHostBindingTransport())

        binding = client.build_host_stream_binding(
            HostStreamBindingRequest(
                binding_id="binding-weather-1",
                descriptor_ref="easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
                endpoint="/tmp/easynet-weather.sock",
                frame_schema=HOST_STREAM_FRAME_SCHEMA,
                cleanup={"mode": "unlink_socket"},
            )
        )
        self.assertEqual(binding.binding_id, "binding-weather-1")
        self.assertEqual(binding.lifecycle["frame_contract_owner"], "daemon_sdk")

        request = client.decode_request(
            HostStreamEnvelope(
                HostStreamEnvelopeRequest(
                    fn="weather.stream",
                    args={"city": "Singapore"},
                    call_id="call-weather-1",
                    caller="easynet:///r/example/user/alice",
                )
            )
        )
        self.assertEqual(request.function, "weather.stream")
        self.assertEqual(request.metadata["source"], "fixture")

        item = client.encode_item(0, {"token": "hello"})
        self.assertEqual(item.frame_type, "item")
        self.assertEqual(item.seq, 0)

        terminal = client.encode_terminal(
            HostStreamTerminalSummary(
                output_hash="sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15",
                frames=1,
            )
        )
        assert terminal.terminal is not None
        self.assertEqual(terminal.output_hash, terminal.terminal.output_hash)

        folded = client.fold_output_hash(
            HostStreamHashState(
                algorithm=HOST_STREAM_HASH_ALGORITHM,
                output_hash="sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                frames=0,
                last_seq=None,
            ),
            0,
            {"token": "hello"},
        )
        self.assertEqual(folded.last_seq, 0)
        self.assertEqual(folded.canonical_json, '{"token":"hello"}')

        with self.assertRaises(SDKError) as caught:
            client.fold_output_hash(
                HostStreamHashState(
                    algorithm=HOST_STREAM_HASH_ALGORITHM,
                    output_hash="sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                    frames=0,
                    last_seq=None,
                ),
                2,
                {"token": "skip"},
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

    def test_python_receipt_executes_shared_projection_conformance_case(self) -> None:
        fetch_case = shared_case("receipt-fetch-carrier.yaml")
        self._require_case_id(fetch_case, "receipt/fetch_carrier")
        self._require_case_action(fetch_case, "build_receipt_fetch_invocation")
        self._require_case_fixture(fetch_case, "receipt-fetch-request.v4.json")
        self._require_case_expectation(
            fetch_case, "invocation_fixture: receipt-fetch-invocation.v4.json"
        )
        self._require_case_expectation(
            fetch_case, "daemon_ability: invocation.history.get"
        )
        self._require_case_expectation(fetch_case, "descriptor_ref_source: request")
        self._require_case_expectation(fetch_case, "selector_cardinality: exactly_one")
        self._require_case_expectation(fetch_case, "direct_ledger_read: false")

        descriptor_delegation_case = shared_case(
            "invocation-descriptor-ref-helper-delegation.yaml"
        )
        self._require_case_id(
            descriptor_delegation_case,
            "invocation/descriptor_ref_helper_delegation",
        )
        for action in (
            "project_descriptor_ref",
            "build_receipt_fetch_invocation",
            "inspect_descriptor_ref_source",
        ):
            self._require_case_action(descriptor_delegation_case, action)
        self._require_case_expectation(
            descriptor_delegation_case, "canonical_helper_owner: axon"
        )
        self._require_case_expectation(
            descriptor_delegation_case,
            "descriptor_ref_source: identity_projection_or_daemon_boundary",
        )
        self._require_case_expectation(
            descriptor_delegation_case,
            "receipt_fetch_descriptor_ref_from_request: true",
        )
        self._require_case_expectation(
            descriptor_delegation_case, "facade_descriptor_concat: false"
        )
        self._require_case_expectation(
            descriptor_delegation_case, "rejects_missing_descriptor_ref: true"
        )

        fetch_request = shared_receipt_fetch_request()
        fetch_draft = build_receipt_fetch_invocation(fetch_request)
        assert_json_equivalent(
            fetch_draft.to_json().encode("utf-8"),
            shared_fixture("receipt-fetch-invocation.v4.json"),
        )
        self.assertEqual(fetch_draft.descriptor_ref, fetch_request.descriptor_ref)
        with self.assertRaises(SDKError) as missing_descriptor_caught:
            build_receipt_fetch_invocation(
                ReceiptFetchRequest(
                    caller_ura=fetch_request.caller_ura,
                    callee_ura=fetch_request.callee_ura,
                    descriptor_ref="",
                    subject_ura=fetch_request.subject_ura,
                    descriptor_version=fetch_request.descriptor_version,
                    nonce_base64=fetch_request.nonce_base64,
                    causal_context=fetch_request.causal_context,
                    request_id=fetch_request.request_id,
                    metadata=fetch_request.metadata,
                )
            )
        self.assertEqual(
            missing_descriptor_caught.exception.code, ErrorCode.INVALID_ARGUMENT
        )
        receipt_source = (ROOT / "sdk/python/easynet_sdk/receipt.py").read_text(
            encoding="utf-8"
        )
        self.assertNotIn("_receipt_ability_descriptor_parts", receipt_source)
        self.assertNotIn('f"{ability_root}/ability/', receipt_source)
        with self.assertRaises(SDKError) as fetch_caught:
            build_receipt_fetch_invocation(
                ReceiptFetchRequest(
                    caller_ura=fetch_request.caller_ura,
                    callee_ura=fetch_request.callee_ura,
                    descriptor_ref=fetch_request.descriptor_ref,
                    subject_ura=fetch_request.subject_ura,
                    descriptor_version=fetch_request.descriptor_version,
                    nonce_base64=fetch_request.nonce_base64,
                    causal_context=fetch_request.causal_context,
                    request_id=fetch_request.request_id,
                    trace_id="trace-1",
                    metadata=fetch_request.metadata,
                )
            )
        self.assertEqual(fetch_caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        receipt_case = shared_case("receipt-projection-causal-ref.yaml")
        self._require_case_id(receipt_case, "receipt/projection_causal_ref")
        for action in (
            "project_receipt_summary",
            "verify_receipt_summary",
            "build_causal_ref",
        ):
            self._require_case_action(receipt_case, action)
        self._require_case_fixture(receipt_case, "receipt.summary.v4.json")
        self._require_case_expectation(receipt_case, "summary_verified: false")
        self._require_case_expectation(
            receipt_case, "verify_summary_claims_cryptographic_validity: false"
        )
        self._require_case_expectation(
            receipt_case, "causal_ref_fixture_result: err_invalid_arg"
        )

        summary = ReceiptSummary.from_json(shared_fixture("receipt.summary.v4.json"))
        self.assertFalse(summary.verified)
        self.assertEqual(summary.state, "completed")
        self.assertIsNone(summary.receipt_ura)

        verification = ReceiptVerification.from_json(
            b'{"verified":false,"method":"summary-only",'
            b'"reason":"full receipt required",'
            b'"metadata":{"source":"sdk_conformance"}}'
        )
        self.assertFalse(verification.verified)
        self.assertEqual(verification.method, "summary-only")

        with self.assertRaises(SDKError) as caught:
            CausalRef.from_json(b'{"metadata":{}}')
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

    def test_python_directory_identity_execute_shared_projection_cases(self) -> None:
        list_case = shared_case("directory-list-pagination.yaml")
        self._require_case_id(list_case, "directory/list_pagination")
        for action in (
            "build_list_devices_invocation",
            "build_list_agents_invocation",
            "project_device_page",
            "project_agent_page",
            "list_devices",
        ):
            self._require_case_action(list_case, action)
        for fixture in (
            "directory-list-devices-request.v4.json",
            "directory-list-agents-request.v4.json",
            "directory-device-page.v4.json",
            "directory-agent-page.v4.json",
        ):
            self._require_case_fixture(list_case, fixture)
        self._require_case_expectation(list_case, "max_page_size: 500")
        self._require_case_expectation(
            list_case,
            "device_invocation_fixture: directory-list-devices-invocation.v4.json",
        )
        self._require_case_expectation(
            list_case,
            "agent_invocation_fixture: directory-list-agents-invocation.v4.json",
        )
        self._require_case_expectation(list_case, "error_code: InvalidArgument")

        directory = DirectoryClient(SharedDirectoryTransport())

        device_page = directory.list_devices(DeviceQuery(shared_directory_query_base(
            "directory-list-devices-request.v4.json"
        )))
        self.assertEqual(device_page.limit, 2)
        self.assertEqual(len(device_page.items), 1)
        self.assertEqual(device_page.metadata["source_ability"], "node.list")

        agent_page = directory.list_agents(AgentQuery(shared_directory_query_base(
            "directory-list-agents-request.v4.json"
        )))
        self.assertEqual(agent_page.limit, 2)
        self.assertEqual(len(agent_page.items), 1)
        self.assertEqual(agent_page.metadata["source_ability"], "agent.list")

        oversized_base = shared_directory_query_base(
            "directory-list-devices-request.v4.json"
        )
        oversized_base = DirectoryQueryBase(
            caller_ura=oversized_base.caller_ura,
            callee_ura=oversized_base.callee_ura,
            subject_ura=oversized_base.subject_ura,
            descriptor_version=oversized_base.descriptor_version,
            nonce_base64=oversized_base.nonce_base64,
            causal_context=oversized_base.causal_context,
            cursor=oversized_base.cursor,
            limit=MAX_DIRECTORY_PAGE_SIZE + 1,
            metadata=oversized_base.metadata,
        )
        with self.assertRaises(SDKError) as caught:
            directory.list_devices(DeviceQuery(oversized_base))
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        fanout_case = shared_case("directory-no-default-fanout.yaml")
        self._require_case_id(fanout_case, "directory/no_default_fanout")
        self._require_case_action(fanout_case, "build_list_abilities_invocation")
        self._require_case_action(fanout_case, "project_ability_page")
        self._require_case_fixture(
            fanout_case, "directory-list-abilities-request.v4.json"
        )
        self._require_case_fixture(fanout_case, "directory-ability-page.v4.json")
        self._require_case_expectation(
            fanout_case, "daemon_ability: meta.list_abilities"
        )
        self._require_case_expectation(
            fanout_case,
            "invocation_fixture: directory-list-abilities-invocation.v4.json",
        )
        self._require_case_expectation(fanout_case, "fanout: none")

        ability_page = directory.list_abilities(shared_ability_query())
        self.assertEqual(ability_page.limit, 2)
        self.assertEqual(len(ability_page.items), 1)
        self.assertEqual(
            ability_page.metadata["source_ability"], "meta.list_abilities"
        )

        resolve_case = shared_case("directory-resolve.yaml")
        self._require_case_id(resolve_case, "directory/resolve")
        self._require_case_action(resolve_case, "build_resolve_invocation")
        self._require_case_action(resolve_case, "project_resolved_ref")
        self._require_case_fixture(resolve_case, "directory-resolve-request.v4.json")
        self._require_case_fixture(resolve_case, "directory-resolved-ref.v4.json")
        self._require_case_expectation(resolve_case, "daemon_ability: namespace.resolve")
        self._require_case_expectation(
            resolve_case, "invocation_fixture: directory-resolve-invocation.v4.json"
        )
        self._require_case_expectation(resolve_case, "fanout: none")
        self._require_case_expectation(
            resolve_case, "route_selection_owner: daemon"
        )

        resolved = directory.resolve(shared_resolve_query())
        self.assertEqual(resolved.kind, "resolved_ref")
        self.assertEqual(
            resolved.ability_ura,
            "easynet:///r/example/ability/device.dev-a.agent.list",
        )

        identity_case = shared_case("identity-ura-descriptor-projection.yaml")
        self._require_case_id(identity_case, "identity/ura_descriptor_projection")
        for action in (
            "project_ura",
            "build_ura",
            "project_descriptor_ref",
            "build_descriptor_ref",
        ):
            self._require_case_action(identity_case, action)
        self._require_case_expectation(identity_case, "grammar_owner: axon")
        self._require_case_expectation(
            identity_case, "fixture: identity.descriptor-ref.v4.json"
        )
        self._require_case_expectation(
            identity_case, "rejects_malformed_descriptor_ref: true"
        )
        self._require_case_expectation(
            identity_case, "rejects_hand_built_invalid_ura: true"
        )

        identity = IdentityClient(SharedIdentityTransport())
        projection = identity.project_descriptor_ref(
            DescriptorRefRequest(
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            )
        )
        self.assertTrue(projection.valid)
        self.assertEqual(projection.metadata["grammar_owner"], "axon")
        self.assertEqual(projection.descriptor_version, "1.0.0")
        parsed = identity.parse_ura(
            "easynet:///r/example/ability/device.dev-a.observe.health"
        )
        self.assertEqual(parsed.kind, "ability")
        ability_ura = identity.owner_ability_ura(
            "easynet:///r/example/device/dev-a", "observe.health"
        )
        self.assertEqual(
            ability_ura, "easynet:///r/example/ability/device.dev-a.observe.health"
        )
        self.assertEqual(
            identity.owner_ura_for_ability(ability_ura),
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(
            identity.canonical_ability_descriptor_ref(ability_ura, "1.0.0"),
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(
            identity.canonical_ability_descriptor_ref(
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            ),
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )

        with self.assertRaises(SDKError) as caught:
            IdentityProjection.from_json(
                b'{"kind":"descriptor_ref","valid":true,'
                b'"profile":"easynet-strict-v2","components":{},'
                b'"metadata":{}}'
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        with self.assertRaises(SDKError) as caught:
            identity.project_identity(IdentityProjectionRequest())
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

    def test_python_publication_executes_shared_carrier_conformance_case(self) -> None:
        publication_case = shared_case("publication-resource-carriers.yaml")
        self._require_case_id(publication_case, "publication/resource_carriers")
        for action in (
            "build_resource_ref",
            "validate_package",
            "build_deploy_invocation",
            "build_unpublish_invocation",
        ):
            self._require_case_action(publication_case, action)
        for fixture in (
            "local-resource-ref-request.v4.json",
            "ability-package-manifest.v4.json",
            "ability-deploy-request.v4.json",
        ):
            self._require_case_fixture(publication_case, fixture)
        self._require_case_expectation(
            publication_case, "resource_ref_fixture: resource-ref.local-fs.v4.json"
        )
        self._require_case_expectation(
            publication_case,
            "package_validation_fixture: package-validation.v4.json",
        )
        self._require_case_expectation(
            publication_case,
            "deploy_invocation_fixture: publication-deploy-invocation.v4.json",
        )
        self._require_case_expectation(
            publication_case,
            "unpublish_invocation_fixture: publication-unpublish-invocation.v4.json",
        )
        self._require_case_expectation(
            publication_case, "deploy_system_ability: ability.deploy"
        )
        self._require_case_expectation(
            publication_case, "unpublish_system_ability: ability.unpublish"
        )
        self._require_case_expectation(publication_case, "rejects_relative_path: true")
        self._require_case_expectation(
            publication_case, "rejects_reserved_namespace: true"
        )
        self._require_case_expectation(
            publication_case, "rejects_incomplete_invocation_tuple: true"
        )

        publication = PublicationClient(SharedPublicationTransport())

        resource_ref = publication.build_local_resource_ref(
            shared_local_resource_ref_request()
        )
        self.assertEqual(resource_ref.namespace, "fs")
        self.assertEqual(resource_ref.capability, "read")
        self.assertEqual(resource_ref.revision, "fs-local-mapping-v1")

        with self.assertRaises(SDKError) as caught:
            publication.build_local_resource_ref(
                LocalResourceRefRequest("tmp/easynet-weather-package", "read")
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        validation = publication.validate_package(
            options=ValidatePackageOptions(manifest=shared_ability_package_manifest())
        )
        self.assertTrue(validation.valid)
        self.assertEqual(validation.manifest.wire_key, "er.weather")
        self.assertEqual(validation.metadata["frame_contract_owner"], "daemon_sdk")

        deploy_request = shared_ability_deploy_request()
        deploy = publication.build_deploy_invocation(deploy_request)
        self.assertEqual(
            deploy.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0",
        )
        self.assertEqual(deploy.metadata["system_ability"], "ability.deploy")

        reserved_ref = ResourceRef(
            resource_ura=deploy_request.resource_ref.resource_ura,
            owner_ura=deploy_request.resource_ref.owner_ura,
            namespace="system",
            capability=deploy_request.resource_ref.capability,
            revision=deploy_request.resource_ref.revision,
            expires_unix_ms=deploy_request.resource_ref.expires_unix_ms,
            display_path=deploy_request.resource_ref.display_path,
        )
        with self.assertRaises(SDKError) as caught:
            publication.build_deploy_invocation(
                AbilityDeployRequest(
                    caller_ura=deploy_request.caller_ura,
                    callee_ura=deploy_request.callee_ura,
                    subject_ura=deploy_request.subject_ura,
                    descriptor_version=deploy_request.descriptor_version,
                    nonce_base64=deploy_request.nonce_base64,
                    causal_context=deploy_request.causal_context,
                    resource_ref=reserved_ref,
                    node_id=deploy_request.node_id,
                    metadata=deploy_request.metadata,
                )
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        with self.assertRaises(SDKError) as caught:
            publication.build_deploy_invocation(
                AbilityDeployRequest(
                    caller_ura="",
                    callee_ura=deploy_request.callee_ura,
                    subject_ura=deploy_request.subject_ura,
                    descriptor_version=deploy_request.descriptor_version,
                    nonce_base64=deploy_request.nonce_base64,
                    causal_context=deploy_request.causal_context,
                    resource_ref=deploy_request.resource_ref,
                    node_id=deploy_request.node_id,
                    metadata=deploy_request.metadata,
                )
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        unpublish = publication.build_unpublish_invocation(
            shared_unpublish_ability_request()
        )
        self.assertEqual(
            unpublish.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0",
        )
        self.assertEqual(unpublish.metadata["system_ability"], "ability.unpublish")

    def test_python_mission_executes_shared_carrier_status_conformance_case(self) -> None:
        mission_case = shared_case("mission-carrier-status.yaml")
        self._require_case_id(mission_case, "mission/carrier_status")
        for action in (
            "build_run_eal_invocation",
            "build_run_file_invocation",
            "build_track_invocation",
            "build_cancel_invocation",
            "project_status",
        ):
            self._require_case_action(mission_case, action)
        for fixture in (
            "mission-run-request.v4.json",
            "mission-run-file-request.v4.json",
            "mission-track-request.v4.json",
            "mission-cancel-request.v4.json",
            "mission-status.v4.json",
        ):
            self._require_case_fixture(mission_case, fixture)
        self._require_case_expectation(
            mission_case, "run_invocation_fixture: mission-run-invocation.v4.json"
        )
        self._require_case_expectation(
            mission_case, "track_invocation_fixture: mission-track-invocation.v4.json"
        )
        self._require_case_expectation(
            mission_case,
            "cancel_invocation_fixture: mission-cancel-invocation.v4.json",
        )
        self._require_case_expectation(mission_case, "run_system_ability: mission.run")
        self._require_case_expectation(
            mission_case, "track_system_ability: mission.track"
        )
        self._require_case_expectation(
            mission_case, "cancel_system_ability: mission.cancel"
        )
        self._require_case_expectation(
            mission_case, "rejects_incomplete_invocation_tuple: true"
        )
        self._require_case_expectation(
            mission_case, "rejects_path_like_mission_id: true"
        )
        self._require_case_expectation(
            mission_case, "child_receipts_only_when_anchored: true"
        )

        mission = MissionClient(SharedMissionTransport())

        run = mission.build_run_eal_invocation(shared_mission_run_request())
        self.assertEqual(
            run.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.mission.run@1.0.0",
        )
        self.assertEqual(run.metadata["system_ability"], "mission.run")

        run_file = mission.build_run_file_invocation(shared_mission_run_file_request())
        self.assertEqual(run_file.descriptor_ref, run.descriptor_ref)
        self.assertEqual(run_file.metadata["system_ability"], "mission.run")

        track = mission.build_track_invocation(shared_mission_track_request())
        self.assertEqual(
            track.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.mission.track@1.0.0",
        )
        self.assertEqual(track.metadata["system_ability"], "mission.track")

        cancel = mission.build_cancel_invocation(shared_mission_cancel_request())
        self.assertEqual(
            cancel.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.mission.cancel@1.0.0",
        )
        self.assertEqual(cancel.metadata["system_ability"], "mission.cancel")

        status = mission.track(shared_mission_track_request())
        self.assertTrue(status.terminal)
        self.assertEqual(status.state, "partial")
        self.assertEqual(
            status.parent_receipt_ura, "easynet:///r/example/receipt/parent"
        )
        self.assertEqual(len(status.child_receipts), 1)
        self.assertEqual(len(status.output_refs), 4)

        run_request = shared_mission_run_request()
        with self.assertRaises(SDKError) as caught:
            mission.build_run_eal_invocation(
                MissionRunRequest(
                    base=MissionCarrierBase(
                        caller_ura="",
                        callee_ura=run_request.base.callee_ura,
                        subject_ura=run_request.base.subject_ura,
                        descriptor_version=run_request.base.descriptor_version,
                        nonce_base64=run_request.base.nonce_base64,
                        causal_context=run_request.base.causal_context,
                        metadata=run_request.base.metadata,
                    ),
                    source=run_request.source,
                    label=run_request.label,
                )
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        with self.assertRaises(SDKError) as caught:
            mission.build_track_invocation(
                MissionTrackRequest(
                    base=shared_mission_carrier_base("mission-track-request.v4.json"),
                    mission_id="/tmp/mission",
                )
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        with self.assertRaises(SDKError) as caught:
            MissionStatus.from_json(shared_mission_status_without_parent_anchor())
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

    def test_python_admin_gateway_executes_shared_carrier_status_conformance_case(self) -> None:
        admin_case = shared_case("admin-gateway-carrier-status.yaml")
        self._require_case_id(admin_case, "admin_gateway/carrier_status")
        for action in (
            "build_agent_list_invocation",
            "build_agent_start_invocation",
            "build_agent_stop_invocation",
            "build_agent_refresh_invocation",
            "build_session_list_invocation",
            "project_gateway_status",
            "project_agent_records",
            "project_agent_lifecycle_result",
        ):
            self._require_case_action(admin_case, action)
        for fixture in (
            "admin-agent-list-request.v4.json",
            "admin-agent-start-request.v4.json",
            "admin-agent-stop-request.v4.json",
            "admin-agent-refresh-request.v4.json",
            "admin-session-list-request.v4.json",
            "gateway-status.v4.json",
            "admin-agent-records.v4.json",
            "admin-agent-lifecycle-result.v4.json",
        ):
            self._require_case_fixture(admin_case, fixture)
        self._require_case_expectation(
            admin_case,
            "agent_start_invocation_fixture: admin-agent-start-invocation.v4.json",
        )
        self._require_case_expectation(
            admin_case,
            "agent_stop_invocation_fixture: admin-agent-stop-invocation.v4.json",
        )
        self._require_case_expectation(
            admin_case,
            "agent_list_invocation_fixture: admin-agent-list-invocation.v4.json",
        )
        self._require_case_expectation(
            admin_case,
            "session_list_invocation_fixture: admin-session-list-invocation.v4.json",
        )
        self._require_case_expectation(
            admin_case, "rejects_incomplete_invocation_tuple: true"
        )
        self._require_case_expectation(
            admin_case, "rejects_system_agent_lifecycle: true"
        )
        self._require_case_expectation(
            admin_case, "preserves_control_only_degraded_state: true"
        )
        self._require_case_expectation(
            admin_case, "pairing_and_device_session_crud: scaffold_only"
        )

        admin = AdminClient(SharedAdminGatewayTransport())

        agent_list = admin.build_agent_list_invocation(shared_admin_agent_list_request())
        self.assertEqual(
            agent_list.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0",
        )
        self.assertEqual(agent_list.metadata["system_ability"], "agent.list")

        agent_start = admin.build_agent_start_invocation(
            shared_admin_agent_start_request()
        )
        self.assertEqual(
            agent_start.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0",
        )
        self.assertEqual(agent_start.metadata["system_ability"], "agent.start")

        agent_stop = admin.build_agent_stop_invocation(shared_admin_agent_stop_request())
        self.assertEqual(
            agent_stop.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0",
        )
        self.assertEqual(agent_stop.metadata["system_ability"], "agent.stop")

        agent_refresh = admin.build_agent_refresh_invocation(
            shared_admin_agent_refresh_request()
        )
        self.assertEqual(
            agent_refresh.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0",
        )
        self.assertEqual(agent_refresh.metadata["system_ability"], "agent.refresh")

        session_list = admin.build_session_list_invocation(
            shared_admin_session_list_request()
        )
        self.assertEqual(
            session_list.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.session.list@1.0.0",
        )
        self.assertEqual(session_list.metadata["system_ability"], "session.list")

        status = admin.gateway_status(AdminGatewayStatusRequest())
        self.assertTrue(status.ready)
        self.assertTrue(status.control_ready)
        self.assertTrue(status.runtime_ready)
        self.assertFalse(status.public_listener_ready)

        agents = admin.list_agents(shared_admin_agent_list_request())
        self.assertEqual(agents.kind, "agent_records")
        self.assertEqual(len(agents.items), 1)
        self.assertEqual(agents.items[0].name, "codex")

        lifecycle = admin.agent_start(shared_admin_agent_start_request())
        self.assertEqual(lifecycle.kind, "agent_lifecycle_result")
        self.assertEqual(lifecycle.state, "ok")
        self.assertFalse(lifecycle.runtime_not_ready)

        start_request = shared_admin_agent_start_request()
        with self.assertRaises(SDKError) as caught:
            admin.build_agent_start_invocation(
                AdminAgentStartRequest(
                    base=AdminCarrierBase(
                        caller_ura="",
                        callee_ura=start_request.base.callee_ura,
                        subject_ura=start_request.base.subject_ura,
                        descriptor_version=start_request.base.descriptor_version,
                        nonce_base64=start_request.base.nonce_base64,
                        causal_context=start_request.base.causal_context,
                        metadata=start_request.base.metadata,
                    ),
                    name=start_request.name,
                    agent_type=start_request.agent_type,
                    model=start_request.model,
                    label=start_request.label,
                )
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        with self.assertRaises(SDKError) as caught:
            admin.build_agent_start_invocation(
                AdminAgentStartRequest(
                    base=start_request.base,
                    name="device",
                    agent_type=start_request.agent_type,
                    model=start_request.model,
                    label=start_request.label,
                )
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        degraded = GatewayStatus.from_json(shared_control_only_gateway_status())
        self.assertFalse(degraded.ready)
        self.assertEqual(degraded.state, "degraded")
        self.assertTrue(degraded.control_ready)
        self.assertFalse(degraded.runtime_ready)

    def test_python_events_executes_shared_directory_stream_conformance_case(self) -> None:
        events_case = shared_case("events-directory-stream.yaml")
        self._require_case_id(events_case, "events/directory_stream")
        for action in (
            "build_directory_subscription_invocation",
            "project_directory_event",
            "project_drop_report",
            "project_terminal",
        ):
            self._require_case_action(events_case, action)
        for fixture in (
            "events-directory-subscription-request.v4.json",
            "event.directory.v4.json",
            "event.directory-drop-report.v4.json",
            "event.directory-terminal.v4.json",
        ):
            self._require_case_fixture(events_case, fixture)
        self._require_case_expectation(
            events_case,
            "subscription_invocation_fixture: events-directory-subscription-invocation.v4.json",
        )
        self._require_case_expectation(
            events_case, "stream_system_ability: federation.subscribe_directory_v2"
        )
        self._require_case_expectation(events_case, "cursor_required: true")
        self._require_case_expectation(
            events_case, "dropped_events_are_first_class: true"
        )
        self._require_case_expectation(
            events_case, "terminal_frame_explicit: true"
        )
        self._require_case_expectation(events_case, "other_event_streams: scaffold_only")

        events = EventClient(SharedEventsTransport())

        subscription = events.build_directory_subscription_invocation(
            shared_events_directory_subscription_request()
        )
        self.assertEqual(
            subscription.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2@1.0.0",
        )
        self.assertEqual(
            subscription.metadata["system_ability"],
            "federation.subscribe_directory_v2",
        )

        directory_event = events.project_directory_event(
            shared_events_projection_input()
        )
        self.assertEqual(directory_event.kind, "directory.agent_advertised")
        self.assertEqual(directory_event.cursor.token, "directory:8")
        self.assertFalse(directory_event.terminal)
        self.assertEqual(
            directory_event.metadata["stream_ability"],
            "federation.subscribe_directory_v2",
        )

        drop_report = events.project_drop_report(shared_events_drop_report_input())
        self.assertEqual(drop_report.kind, "directory.drop_report")
        self.assertEqual(drop_report.dropped_count, 4)
        self.assertEqual(drop_report.reconnect_after_ms, 1000)

        terminal = events.project_terminal(shared_events_terminal_input())
        self.assertEqual(terminal.kind, "directory.terminal")
        self.assertTrue(terminal.terminal)
        self.assertEqual(terminal.resume_token, "terminal")

        request = shared_events_directory_subscription_request()
        with self.assertRaises(SDKError) as caught:
            events.build_directory_subscription_invocation(
                EventsDirectorySubscriptionRequest(
                    base=EventsCarrierBase(
                        caller_ura="",
                        callee_ura=request.base.callee_ura,
                        subject_ura=request.base.subject_ura,
                        descriptor_version=request.base.descriptor_version,
                        nonce_base64=request.base.nonce_base64,
                        causal_context=request.base.causal_context,
                        metadata=request.base.metadata,
                    ),
                    realm=request.realm,
                    agent_ura=request.agent_ura,
                    resume_cursor=request.resume_cursor,
                    heartbeat_interval_ms=request.heartbeat_interval_ms,
                )
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        with self.assertRaises(SDKError) as caught:
            EventFrame.from_json(
                shared_events_frame_without_cursor_token("event.directory.v4.json")
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        with self.assertRaises(SDKError) as caught:
            EventFrame.from_json(shared_events_drop_report_without_dropped_count())
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        with self.assertRaises(SDKError) as caught:
            EventFrame.from_json(shared_events_terminal_without_terminal_flag())
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

    def test_python_surface_executes_shared_page_carrier_conformance_case(self) -> None:
        surface_case = shared_case("surface-page-carriers.yaml")
        self._require_case_id(surface_case, "surface/page_carriers")
        for action in (
            "build_surface_list_pages_invocation",
            "build_surface_create_page_invocation",
            "build_surface_delete_page_invocation",
            "build_surface_manifest_invocation",
            "project_surface_page_page",
            "project_surface_manifest",
        ):
            self._require_case_action(surface_case, action)
        for fixture in (
            "surface-list-pages-request.v4.json",
            "surface-create-page-request.v4.json",
            "surface-delete-page-request.v4.json",
            "surface-manifest-request.v4.json",
            "surface-page-page.v4.json",
            "surface-manifest.v4.json",
        ):
            self._require_case_fixture(surface_case, fixture)
        for ability in ("pages.list", "pages.publish", "pages.get", "pages.unpublish"):
            self._require_case_literal(surface_case, f"- {ability}")
        self._require_case_expectation(
            surface_case, "backend_rendering_owned_by_sdk: false"
        )
        self._require_case_expectation(
            surface_case, "direct_filesystem_page_transport: false"
        )

        surface = SurfaceClient(SharedSurfaceTransport())

        list_draft = surface.build_list_pages_invocation(
            shared_surface_list_pages_request()
        )
        self.assertEqual(
            list_draft.descriptor_ref,
            "easynet:///r/example/ability/alice.pages.pages.list@1.0.0",
        )
        self.assertEqual(list_draft.metadata["system_ability"], "pages.list")

        create_draft = surface.build_create_page_invocation(
            shared_surface_create_page_request()
        )
        self.assertEqual(
            create_draft.descriptor_ref,
            "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0",
        )
        self.assertEqual(create_draft.metadata["system_ability"], "pages.publish")

        delete_draft = surface.build_delete_page_invocation(
            shared_surface_delete_page_request()
        )
        self.assertEqual(
            delete_draft.descriptor_ref,
            "easynet:///r/example/ability/alice.pages.pages.unpublish@1.0.0",
        )
        self.assertEqual(delete_draft.metadata["system_ability"], "pages.unpublish")

        manifest_draft = surface.build_manifest_invocation(
            shared_surface_manifest_request()
        )
        self.assertEqual(
            manifest_draft.descriptor_ref,
            "easynet:///r/example/ability/alice.pages.pages.get@1.0.0",
        )
        self.assertEqual(manifest_draft.metadata["system_ability"], "pages.get")

        page = surface.list_pages(shared_surface_list_pages_request())
        self.assertEqual(page.kind, "surface_page_page")
        self.assertEqual(page.source, "pages_read_model")
        self.assertEqual(len(page.items), 1)
        self.assertEqual(
            page.items[0].surface_ref, "easynet:///r/example/resource/alice.docs"
        )

        manifest = surface.surface_manifest(shared_surface_manifest_request())
        self.assertEqual(manifest.kind, "surface_manifest")
        self.assertEqual(manifest.page.page_id, "docs")
        self.assertEqual(manifest.entrypoint["kind"], "public_page_ref")

        create_request = shared_surface_create_page_request()
        with self.assertRaises(SDKError) as caught:
            surface.build_create_page_invocation(
                SurfaceCreatePageRequest(
                    base=SurfaceCarrierBase(
                        caller_ura="",
                        callee_ura=create_request.base.callee_ura,
                        subject_ura=create_request.base.subject_ura,
                        descriptor_version=create_request.base.descriptor_version,
                        nonce_base64=create_request.base.nonce_base64,
                        causal_context=create_request.base.causal_context,
                        metadata=create_request.base.metadata,
                    ),
                    project_id=create_request.project_id,
                    folder=create_request.folder,
                    visibility=create_request.visibility,
                )
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        with self.assertRaises(SDKError) as caught:
            surface.build_create_page_invocation(
                SurfaceCreatePageRequest(
                    base=create_request.base,
                    project_id=create_request.project_id,
                    folder="tmp/easynet-pages-docs",
                    visibility=create_request.visibility,
                )
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        with self.assertRaises(SDKError) as caught:
            SurfacePagePage.from_json(shared_surface_page_page_with_oversized_limit())
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        with self.assertRaises(SDKError) as caught:
            SurfaceManifest.from_json(shared_surface_manifest_without_entrypoint())
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

    def test_python_compatibility_executes_shared_openai_carrier_conformance_case(self) -> None:
        compatibility_case = shared_case("compatibility-openai-carrier-projection.yaml")
        self._require_case_id(
            compatibility_case, "compatibility/openai_carrier_projection"
        )
        for action in (
            "build_list_models_invocation",
            "build_chat_completion_invocation",
            "build_stream_chat_completion_invocation",
            "project_model_page",
            "project_chat_completion",
            "project_chat_stream",
            "project_file_upload",
            "project_file",
            "project_file_delete_result",
        ):
            self._require_case_action(compatibility_case, action)
        for fixture in (
            "compatibility-list-models-request.v4.json",
            "compatibility-list-models-invocation.v4.json",
            "compatibility-chat-completion-request.v4.json",
            "compatibility-chat-completion-invocation.v4.json",
            "compatibility-stream-chat-completion-request.v4.json",
            "compatibility-stream-chat-completion-invocation.v4.json",
            "compatibility-model-page.v4.json",
            "compatibility-chat-completion.v4.json",
            "compatibility-chat-stream.v4.json",
            "compatibility-file-upload-request.v4.json",
            "compatibility-file-request.v4.json",
            "compatibility-file.v4.json",
            "compatibility-file-delete-request.v4.json",
            "compatibility-file-delete-result.v4.json",
        ):
            self._require_case_fixture(compatibility_case, fixture)
        for expectation in (
            "rejects_provider_nickname_models: true",
            "rejects_unary_stream_true: true",
            "files_api: file_wrapper_projection",
            "openai_files_daemon_ability_required: false",
            "product_http_auth_and_sse_fanout: product_owned",
        ):
            self._require_case_expectation(compatibility_case, expectation)

        compatibility = CompatibilityClient(SharedCompatibilityTransport())

        list_draft = compatibility.build_list_models_invocation(
            shared_compatibility_list_models_request()
        )
        self.assertEqual(
            list_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.openai.list_models@1.0.0",
        )
        self.assertEqual(list_draft.metadata["system_ability"], "openai.list_models")

        chat_draft = compatibility.build_chat_completion_invocation(
            shared_compatibility_chat_completion_request()
        )
        self.assertEqual(
            chat_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0",
        )
        self.assertEqual(
            chat_draft.metadata["system_ability"], "openai.chat_completions"
        )

        stream_draft = compatibility.build_stream_chat_completion_invocation(
            shared_compatibility_stream_chat_completion_request()
        )
        self.assertEqual(
            stream_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0",
        )
        self.assertEqual(
            stream_draft.metadata["system_ability"], "openai.chat_completions"
        )

        models = compatibility.list_models(shared_compatibility_list_models_request())
        self.assertEqual(models.kind, "model_page")
        self.assertEqual(len(models.data), 1)
        self.assertEqual(
            models.data[0].ability_ref,
            "easynet:///r/example/ability/alice.codex.chat",
        )

        chat = compatibility.create_chat_completion(
            shared_compatibility_chat_completion_request()
        )
        self.assertEqual(chat.kind, "chat_completion")
        self.assertEqual(chat.model, "easynet:///r/example/ability/alice.codex.chat")
        self.assertEqual(len(chat.choices), 1)

        stream = compatibility.stream_chat_completion(
            shared_compatibility_stream_chat_completion_request()
        )
        self.assertEqual(stream.kind, "chat_completion_stream")
        self.assertTrue(stream.stream)
        self.assertEqual(stream.done_sentinel, "[DONE]")
        self.assertEqual(len(stream.items), 1)

        uploaded = compatibility.project_file_upload(
            shared_compatibility_file_upload_request()
        )
        assert_json_equivalent(
            json.dumps(uploaded.__dict__, separators=(",", ":"), sort_keys=True).encode(
                "utf-8"
            ),
            shared_fixture("compatibility-file.v4.json"),
        )

        file = compatibility.project_file(shared_compatibility_file_request())
        assert_json_equivalent(
            json.dumps(file.__dict__, separators=(",", ":"), sort_keys=True).encode(
                "utf-8"
            ),
            shared_fixture("compatibility-file.v4.json"),
        )

        deleted = compatibility.project_file_delete_result(
            shared_compatibility_file_delete_request()
        )
        assert_json_equivalent(
            json.dumps(deleted.__dict__, separators=(",", ":"), sort_keys=True).encode(
                "utf-8"
            ),
            shared_fixture("compatibility-file-delete-result.v4.json"),
        )

        nickname_model = shared_compatibility_chat_completion_request()
        with self.assertRaises(SDKError) as caught:
            compatibility.build_chat_completion_invocation(
                CompatibilityChatCompletionRequest(
                    nickname_model.base,
                    {
                        **dict(nickname_model.request),
                        "model": "gpt-4o-mini",
                    },
                )
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        unary_stream = shared_compatibility_chat_completion_request()
        with self.assertRaises(SDKError) as caught:
            compatibility.build_chat_completion_invocation(
                CompatibilityChatCompletionRequest(
                    unary_stream.base,
                    {
                        **dict(unary_stream.request),
                        "stream": True,
                    },
                )
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

    def test_python_wrappers_execute_shared_projection_conformance_case(self) -> None:
        wrapper_case = shared_case("wrapper-profile-records.yaml")
        self._require_case_id(wrapper_case, "wrappers/profile_records")
        for action in (
            "project_file_record",
            "project_terminal_session",
            "project_remote_desktop_session",
            "project_browser_session",
            "project_media_session",
        ):
            self._require_case_action(wrapper_case, action)
        for fixture in (
            "wrapper-file-record.v4.json",
            "wrapper-terminal-session.v4.json",
            "wrapper-remote-desktop-session.v4.json",
            "wrapper-browser-session.v4.json",
            "wrapper-media-session.v4.json",
        ):
            self._require_case_fixture(wrapper_case, fixture)
        self._require_case_expectation(wrapper_case, "execution_transport_owner: runtime_core")
        self._require_case_expectation(wrapper_case, "product_http_websocket_owner: backend")
        self._require_case_expectation(wrapper_case, "rejects_invalid_owner_ura: true")
        self._require_case_expectation(wrapper_case, "rejects_missing_session_state: true")

        file = WrapperFileRecord.from_json(shared_fixture("wrapper-file-record.v4.json"))
        self.assertEqual(file.kind, "file_record")
        self.assertEqual(file.metadata["source"], "wrappers.file_record")

        terminal = WrapperTerminalSessionRecord.from_json(
            shared_fixture("wrapper-terminal-session.v4.json")
        )
        self.assertEqual(terminal.kind, "terminal_session")
        self.assertEqual(terminal.terminal_ref, "terminal-main")

        remote = WrapperRemoteDesktopSessionRecord.from_json(
            shared_fixture("wrapper-remote-desktop-session.v4.json")
        )
        self.assertEqual(remote.kind, "remote_desktop_session")
        self.assertEqual(remote.display_ref, "display-main")

        browser = WrapperBrowserSessionRecord.from_json(
            shared_fixture("wrapper-browser-session.v4.json")
        )
        self.assertEqual(browser.kind, "browser_session")
        self.assertEqual(browser.state, "starting")

        media = WrapperMediaSessionRecord.from_json(
            shared_fixture("wrapper-media-session.v4.json")
        )
        self.assertEqual(media.kind, "media_session")
        self.assertEqual(media.media_kind, "voice")
        self.assertEqual(media.stream_ref, "stream-voice-1")

        client = WrapperClient()
        with self.assertRaises(SDKError) as caught:
            client.project_file_record(
                WrapperFileRecordRequest(
                    file_ref=file.file_ref,
                    owner_ura="not-a-ura",
                    content_type=file.content_type,
                )
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

        with self.assertRaises(SDKError) as caught:
            client.project_terminal_session(
                WrapperTerminalSessionRequest(
                    session_id=terminal.session_id,
                    owner_ura=terminal.owner_ura,
                    state="",
                )
            )
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

    def test_python_memc_executes_shared_profile_exclusivity_conformance_case(self) -> None:
        memc_case = shared_case("memc-profile-exclusivity.yaml")
        self._require_case_id(memc_case, "memc/profile_exclusivity")
        self._require_case_action(memc_case, "inspect_public_api")
        self._require_case_expectation(memc_case, "duplicate_profile_owners: 0")

        audits = [
            (
                "runtime_core",
                Client,
                {
                    "close": "runtime_core.client.close",
                    "feature_discovery": "runtime_core.feature_discovery",
                    "require_abi": "runtime_core.require_abi",
                },
            ),
            (
                "runtime_core",
                DaemonControl,
                {
                    "attach": "runtime_core.daemon.attach",
                    "connect_local": "runtime_core.daemon.connect_local",
                    "discover": "runtime_core.daemon.discover",
                    "start": "runtime_core.daemon.start",
                },
            ),
            (
                "runtime_core",
                DaemonHandle,
                {
                    "detach": "runtime_core.daemon_handle.detach",
                    "open_runtime": "runtime_core.daemon_handle.open_runtime",
                    "status": "runtime_core.daemon_handle.status",
                    "stop": "runtime_core.daemon_handle.stop",
                },
            ),
            (
                "runtime_core",
                RuntimeClient,
                {
                    "await_result": "runtime_core.invocation.await",
                    "cancel": "runtime_core.invocation.cancel",
                    "close": "runtime_core.runtime_client.close",
                    "close_handle": "runtime_core.invocation.close_handle",
                    "events": "runtime_core.invocation.events",
                    "invoke": "runtime_core.invocation.invoke",
                    "invoke_stream": "runtime_core.invocation.invoke_stream",
                    "open_bidi": "runtime_core.invocation.open_bidi",
                    "prepare": "runtime_core.invocation.prepare",
                    "prepare_builder": "runtime_core.invocation.prepare_builder",
                    "submit_signed": "runtime_core.invocation.submit_signed",
                },
            ),
            (
                "runtime_core",
                HealthClient,
                {
                    "runtime_health": "runtime_core.health.runtime_health",
                },
            ),
            (
                "directory_identity",
                DirectoryClient,
                {
                    "build_directory_subscription_invocation": "directory_identity.directory.build_subscription_invocation",
                    "close": "directory_identity.directory.close",
                    "list_abilities": "directory_identity.directory.list_abilities",
                    "list_agents": "directory_identity.directory.list_agents",
                    "list_devices": "directory_identity.directory.list_devices",
                    "resolve": "directory_identity.directory.resolve",
                    "subscribe_directory": "directory_identity.directory.subscribe",
                },
            ),
            (
                "directory_identity",
                IdentityClient,
                {
                    "build_resource_ref": "directory_identity.identity.build_resource_ref",
                    "canonical_ability_descriptor_ref": "directory_identity.identity.canonical_ability_descriptor_ref",
                    "close": "directory_identity.identity.close",
                    "list_signing_keys": "directory_identity.identity.list_signing_keys",
                    "owner_ability_ura": "directory_identity.identity.owner_ability_ura",
                    "owner_ura_for_ability": "directory_identity.identity.owner_ura_for_ability",
                    "parse_ura": "directory_identity.identity.parse_ura",
                    "project_descriptor_ref": "directory_identity.identity.project_descriptor_ref",
                    "project_identity": "directory_identity.identity.project_identity",
                    "register_signing_key": "directory_identity.identity.register_signing_key",
                    "revoke_signing_key": "directory_identity.identity.revoke_signing_key",
                    "signer": "directory_identity.identity.signer",
                },
            ),
            (
                "receipt",
                ReceiptClient,
                {
                    "build_fetch_invocation": "receipt.build_fetch_invocation",
                    "causal_ref": "receipt.causal_ref",
                    "close": "receipt.close",
                    "fetch": "receipt.fetch",
                    "project": "receipt.project",
                    "verify": "receipt.verify",
                    "verify_chain": "receipt.verify_chain",
                },
            ),
            (
                "publication",
                PublicationClient,
                {
                    "build_deploy_invocation": "publication.build_deploy_invocation",
                    "build_local_resource_ref": "publication.build_local_resource_ref",
                    "build_unpublish_invocation": "publication.build_unpublish_invocation",
                    "close": "publication.close",
                    "deploy_ability": "publication.deploy_ability",
                    "disable_ability_impl": "publication.disable_ability_impl",
                    "enable_ability_impl": "publication.enable_ability_impl",
                    "install_plugin": "publication.install_plugin",
                    "list_abilities": "publication.list_abilities",
                    "show_ability": "publication.show_ability",
                    "unpublish_ability": "publication.unpublish_ability",
                    "validate_package": "publication.validate_package",
                },
            ),
            (
                "host_binding",
                HostBindingClient,
                {
                    "build_host_stream_binding": "host_binding.build_host_stream_binding",
                    "close": "host_binding.close",
                    "decode_request": "host_binding.decode_request",
                    "encode_error": "host_binding.encode_error",
                    "encode_item": "host_binding.encode_item",
                    "encode_terminal": "host_binding.encode_terminal",
                    "fold_output_hash": "host_binding.fold_output_hash",
                },
            ),
            (
                "mission",
                MissionClient,
                {
                    "build_cancel_invocation": "mission.build_cancel_invocation",
                    "build_run_eal_invocation": "mission.build_run_eal_invocation",
                    "build_run_file_invocation": "mission.build_run_file_invocation",
                    "build_track_invocation": "mission.build_track_invocation",
                    "cancel": "mission.cancel",
                    "close": "mission.close",
                    "events": "mission.events",
                    "run_eal": "mission.run_eal",
                    "run_file": "mission.run_file",
                    "track": "mission.track",
                },
            ),
            (
                "admin_gateway",
                AdminClient,
                {
                    "agent_refresh": "admin_gateway.agent.refresh",
                    "agent_start": "admin_gateway.agent.start",
                    "agent_stop": "admin_gateway.agent.stop",
                    "build_agent_list_invocation": "admin_gateway.agent.build_list_invocation",
                    "build_agent_refresh_invocation": "admin_gateway.agent.build_refresh_invocation",
                    "build_agent_start_invocation": "admin_gateway.agent.build_start_invocation",
                    "build_agent_stop_invocation": "admin_gateway.agent.build_stop_invocation",
                    "build_session_list_invocation": "admin_gateway.session.build_list_invocation",
                    "close": "admin_gateway.close",
                    "create_device_session": "admin_gateway.session.create",
                    "create_pairing": "admin_gateway.pairing.create",
                    "delete_device_session": "admin_gateway.session.delete",
                    "gateway_status": "admin_gateway.gateway.status",
                    "join_hub": "admin_gateway.hub.join",
                    "leave_hub": "admin_gateway.hub.leave",
                    "list_agents": "admin_gateway.agent.list",
                    "list_device_sessions": "admin_gateway.session.list",
                    "pairing_preflight": "admin_gateway.pairing.preflight",
                    "revoke_device": "admin_gateway.device.revoke",
                    "validate_pairing": "admin_gateway.pairing.validate",
                    "verify_device_credential": "admin_gateway.device.verify_credential",
                },
            ),
            (
                "events",
                EventClient,
                {
                    "build_device_subscription_invocation": "events.build_device_subscription_invocation",
                    "build_directory_subscription_invocation": "events.build_directory_subscription_invocation",
                    "build_invocation_subscription_invocation": "events.build_invocation_subscription_invocation",
                    "build_session_subscription_invocation": "events.build_session_subscription_invocation",
                    "close": "events.close",
                    "list_device_events": "events.list_device_events",
                    "project_directory_event": "events.project_directory_event",
                    "project_drop_report": "events.project_drop_report",
                    "project_terminal": "events.project_terminal",
                    "subscribe_devices": "events.subscribe_devices",
                    "subscribe_directory": "events.subscribe_directory",
                    "subscribe_invocations": "events.subscribe_invocations",
                    "subscribe_sessions": "events.subscribe_sessions",
                },
            ),
            (
                "surface",
                SurfaceClient,
                {
                    "build_create_page_invocation": "surface.build_create_page_invocation",
                    "build_delete_page_invocation": "surface.build_delete_page_invocation",
                    "build_health_invocation": "surface.build_health_invocation",
                    "build_list_pages_invocation": "surface.build_list_pages_invocation",
                    "build_manifest_invocation": "surface.build_manifest_invocation",
                    "close": "surface.close",
                    "create_page": "surface.create_page",
                    "delete_page": "surface.delete_page",
                    "list_pages": "surface.list_pages",
                    "public_page_ref": "surface.public_page_ref",
                    "surface_health": "surface.health",
                    "surface_manifest": "surface.manifest",
                    "surface_status": "surface.status",
                },
            ),
            (
                "compatibility",
                CompatibilityClient,
                {
                    "build_chat_completion_invocation": "compatibility.chat.build_completion_invocation",
                    "build_file_delete_invocation": "compatibility.file.build_delete_invocation",
                    "build_file_get_invocation": "compatibility.file.retrieve",
                    "build_file_retrieve_invocation": "compatibility.file.retrieve",
                    "build_file_upload_invocation": "compatibility.file.build_upload_invocation",
                    "build_list_models_invocation": "compatibility.models.build_list_invocation",
                    "build_stream_chat_completion_invocation": "compatibility.chat.build_stream_invocation",
                    "close": "compatibility.close",
                    "create_chat_completion": "compatibility.chat.create_completion",
                    "delete_file": "compatibility.file.delete",
                    "get_file": "compatibility.file.retrieve",
                    "list_models": "compatibility.models.list",
                    "project_file": "compatibility.file.project",
                    "project_file_delete_result": "compatibility.file.project_delete_result",
                    "project_file_upload": "compatibility.file.project_upload",
                    "retrieve_file": "compatibility.file.retrieve",
                    "stream_chat_completion": "compatibility.chat.stream_completion",
                    "upload_file": "compatibility.file.upload",
                },
            ),
            (
                "wrappers",
                WrapperClient,
                {
                    "build_browser_session_invocation": "wrappers.browser.build_session_invocation",
                    "build_file_transfer_invocation": "wrappers.file.build_transfer_invocation",
                    "build_media_session_invocation": "wrappers.media.build_session_invocation",
                    "build_remote_desktop_session_invocation": "wrappers.remote_desktop.build_session_invocation",
                    "build_terminal_session_invocation": "wrappers.terminal.build_session_invocation",
                    "close": "wrappers.close",
                    "project_browser_session": "wrappers.browser.project_session",
                    "project_file_record": "wrappers.file.project_record",
                    "project_media_session": "wrappers.media.project_session",
                    "project_remote_desktop_session": "wrappers.remote_desktop.project_session",
                    "project_terminal_session": "wrappers.terminal.project_session",
                    "start_browser_session": "wrappers.browser.start_session",
                    "start_media_session": "wrappers.media.start_session",
                    "start_remote_desktop_session": "wrappers.remote_desktop.start_session",
                    "start_terminal_session": "wrappers.terminal.start_session",
                    "transfer_file": "wrappers.file.transfer",
                },
            ),
        ]

        unmapped, duplicate_owners = audit_shared_profile_ownership(audits)
        self.assertEqual([], unmapped)
        self.assertEqual([], duplicate_owners)

    def test_python_memc_executes_shared_consumer_coverage_conformance_case(self) -> None:
        coverage_case = shared_case("memc-consumer-coverage.yaml")
        self._require_case_id(coverage_case, "memc/consumer_coverage")
        self._require_case_action(coverage_case, "inspect_consumer_coverage")
        self._require_case_expectation(coverage_case, "raw_lower_layer_dependency: false")
        for consumer in (
            "backend_hub",
            "easyremote",
            "cli",
            "desktop_gui",
            "third_party_host_app",
            "future_bindings",
        ):
            self._require_case_literal(coverage_case, f"- {consumer}")
        for forbidden in (
            "axon_sdk_proto",
            "c_abi_direct",
            "raw_daemon_socket",
            "control_frame_product_call",
            "cli_subprocess",
            "easyremote_dependency",
            "product_local_daemon_transport",
        ):
            self._require_case_literal(coverage_case, f"- {forbidden}")

        requirements = [
            (
                "backend_hub",
                "runtime_core",
                (
                    (Client, ("require_abi", "feature_discovery")),
                    (RuntimeClient, ("invoke", "invoke_stream", "open_bidi", "await_result", "cancel")),
                    (HealthClient, ("runtime_health",)),
                ),
            ),
            (
                "backend_hub",
                "directory_identity",
                (
                    (DirectoryClient, ("resolve", "list_devices", "list_agents", "list_abilities")),
                    (IdentityClient, ("project_descriptor_ref", "build_resource_ref")),
                ),
            ),
            (
                "backend_hub",
                "receipt",
                ((ReceiptClient, ("fetch", "project", "verify", "causal_ref")),),
            ),
            (
                "backend_hub",
                "events",
                ((EventClient, ("subscribe_directory", "subscribe_invocations", "list_device_events", "project_drop_report")),),
            ),
            (
                "backend_hub",
                "admin_gateway",
                ((AdminClient, ("gateway_status", "list_agents", "list_device_sessions", "join_hub", "leave_hub")),),
            ),
            (
                "backend_hub",
                "surface",
                ((SurfaceClient, ("list_pages", "create_page", "delete_page", "surface_manifest", "public_page_ref", "surface_health")),),
            ),
            (
                "backend_hub",
                "compatibility",
                ((CompatibilityClient, ("list_models", "create_chat_completion", "stream_chat_completion", "upload_file", "retrieve_file", "delete_file")),),
            ),
            (
                "backend_hub",
                "publication",
                ((PublicationClient, ("list_abilities", "show_ability", "build_deploy_invocation")),),
            ),
            (
                "backend_hub",
                "wrappers",
                ((WrapperClient, ("transfer_file", "start_terminal_session", "start_remote_desktop_session", "start_browser_session", "start_media_session")),),
            ),
            (
                "easyremote",
                "runtime_core",
                (
                    (DaemonControl, ("start", "attach", "connect_local")),
                    (RuntimeClient, ("prepare", "submit_signed", "invoke", "invoke_stream", "open_bidi")),
                ),
            ),
            (
                "easyremote",
                "directory_identity",
                (
                    (DirectoryClient, ("resolve", "list_abilities")),
                    (IdentityClient, ("build_resource_ref", "signer", "register_signing_key", "list_signing_keys")),
                ),
            ),
            (
                "easyremote",
                "publication",
                ((PublicationClient, ("build_local_resource_ref", "deploy_ability", "list_abilities", "show_ability", "enable_ability_impl", "disable_ability_impl")),),
            ),
            (
                "easyremote",
                "host_binding",
                ((HostBindingClient, ("build_host_stream_binding", "decode_request", "encode_item", "encode_error", "encode_terminal", "fold_output_hash")),),
            ),
            (
                "easyremote",
                "mission",
                ((MissionClient, ("build_run_eal_invocation", "run_eal", "run_file", "track", "cancel", "events")),),
            ),
            (
                "easyremote",
                "admin_gateway",
                ((AdminClient, ("gateway_status", "list_agents", "agent_start", "agent_refresh")),),
            ),
            (
                "cli",
                "runtime_core",
                (
                    (DaemonControl, ("discover", "start", "attach")),
                    (RuntimeClient, ("invoke", "invoke_stream", "open_bidi")),
                ),
            ),
            (
                "cli",
                "directory_identity",
                ((DirectoryClient, ("resolve", "list_devices", "list_agents", "list_abilities")),),
            ),
            (
                "cli",
                "publication",
                ((PublicationClient, ("validate_package", "deploy_ability", "install_plugin", "unpublish_ability")),),
            ),
            (
                "cli",
                "host_binding",
                ((HostBindingClient, ("build_host_stream_binding", "fold_output_hash")),),
            ),
            (
                "cli",
                "mission",
                ((MissionClient, ("run_eal", "run_file", "track", "cancel")),),
            ),
            (
                "cli",
                "admin_gateway",
                ((AdminClient, ("gateway_status", "join_hub", "leave_hub", "list_agents")),),
            ),
            (
                "cli",
                "wrappers",
                ((WrapperClient, ("transfer_file", "start_terminal_session")),),
            ),
            (
                "desktop_gui",
                "runtime_core",
                (
                    (DaemonControl, ("start", "attach", "connect_local")),
                    (RuntimeClient, ("invoke", "invoke_stream", "open_bidi")),
                    (HealthClient, ("runtime_health",)),
                ),
            ),
            (
                "desktop_gui",
                "directory_identity",
                ((DirectoryClient, ("list_devices", "list_agents", "list_abilities", "resolve")),),
            ),
            (
                "desktop_gui",
                "wrappers",
                ((WrapperClient, ("start_terminal_session", "start_remote_desktop_session")),),
            ),
            (
                "third_party_host_app",
                "runtime_core",
                ((RuntimeClient, ("invoke", "prepare", "submit_signed")),),
            ),
            (
                "third_party_host_app",
                "directory_identity",
                (
                    (DirectoryClient, ("resolve", "list_abilities")),
                    (IdentityClient, ("build_resource_ref", "project_descriptor_ref")),
                ),
            ),
            (
                "third_party_host_app",
                "publication",
                ((PublicationClient, ("build_local_resource_ref", "validate_package", "deploy_ability")),),
            ),
            (
                "third_party_host_app",
                "host_binding",
                ((HostBindingClient, ("build_host_stream_binding", "decode_request", "encode_terminal", "fold_output_hash")),),
            ),
        ]

        missing, duplicates = audit_shared_consumer_coverage(requirements)
        self.assertEqual([], missing)
        self.assertEqual([], duplicates)

    def test_python_memc_executes_shared_no_core_bloat_conformance_case(self) -> None:
        no_bloat_case = shared_case("memc-no-core-bloat.yaml")
        self._require_case_id(no_bloat_case, "memc/no_core_bloat")
        self._require_case_action(no_bloat_case, "inspect_runtime_core_surface")
        for expectation in (
            "publication_package_building: false",
            "python_decorators: false",
            "backend_dtos: false",
            "cli_command_text: false",
            "one_method_per_ability_required_api: false",
        ):
            self._require_case_expectation(no_bloat_case, expectation)
        for scope in (
            "lifecycle",
            "invocation",
            "signing_material",
            "unary_stream_bidi",
            "health_error",
            "process_safe_client_behavior",
        ):
            self._require_case_literal(no_bloat_case, f"- {scope}")

        forbidden_tokens = (
            "Publication",
            "DeployAbility",
            "ValidatePackage",
            "InstallPlugin",
            "HostBinding",
            "Mission",
            "Admin",
            "Gateway",
            "Surface",
            "Compatibility",
            "Wrapper",
            "Backend",
            "CLICommand",
            "Decorator",
            "InvokeAbility",
            "CallAbility",
        )
        for token in forbidden_tokens:
            self._require_case_literal(no_bloat_case, f"- {token}")

        surfaces = (
            Client,
            DaemonControl,
            DaemonHandle,
            RuntimeClient,
            HealthClient,
            InvocationBuilder,
            PreparedInvocation,
            SignedInvocation,
            StreamHandle,
            BidiSession,
        )
        core_files = (
            "client.py",
            "runtime.py",
            "daemon.py",
            "health.py",
            "invocation.py",
            "signing.py",
            "stream.py",
            "bidi.py",
            "errors.py",
        )
        violations = audit_shared_no_core_bloat(
            surfaces,
            tuple(ROOT.joinpath("sdk/python/easynet_sdk", name) for name in core_files),
            forbidden_tokens,
        )
        self.assertEqual([], violations)

    def _require_case_id(self, raw: str, case_id: str) -> None:
        self._require_case_literal(raw, f"id: {case_id}")

    def _require_case_action(self, raw: str, action: str) -> None:
        self._require_case_literal(raw, f"action: {action}")

    def _require_case_fixture(self, raw: str, fixture: str) -> None:
        self._require_case_literal(raw, f"fixture: {fixture}")

    def _require_case_expectation(self, raw: str, expected: str) -> None:
        self._require_case_literal(raw, expected)

    def _require_case_literal(self, raw: str, expected: str) -> None:
        self.assertIn(expected, raw)


def audit_shared_profile_ownership(audits):
    unmapped = []
    operation_owners = {}
    for owner, cls, operations in audits:
        for name, _ in inspect.getmembers(cls, inspect.isfunction):
            if name.startswith("_"):
                continue
            operation = operations.get(name)
            if operation is None:
                unmapped.append(f"{cls.__name__}.{name}")
                continue
            operation_owners.setdefault(operation, set()).add(owner)

    duplicate_owners = []
    for operation, owners in operation_owners.items():
        if len(owners) > 1:
            duplicate_owners.append(
                f"{operation} owned by {', '.join(sorted(owners))}"
            )
    return sorted(unmapped), sorted(duplicate_owners)


def audit_shared_consumer_coverage(requirements):
    missing = []
    duplicates = []
    seen = set()
    for consumer, profile, surfaces in requirements:
        key = f"{consumer}/{profile}"
        if key in seen:
            duplicates.append(key)
        seen.add(key)
        if not surfaces:
            missing.append(f"{key} has no public SDK surface")
            continue
        for cls, methods in surfaces:
            public_methods = {
                name
                for name, _ in inspect.getmembers(cls, inspect.isfunction)
                if not name.startswith("_")
            }
            for method in methods:
                if method not in public_methods:
                    missing.append(f"{key} missing {cls.__name__}.{method}")
    return sorted(missing), sorted(duplicates)


def audit_shared_no_core_bloat(surfaces, core_files, forbidden_tokens):
    violations = []
    for cls in surfaces:
        for name, _ in inspect.getmembers(cls, inspect.isfunction):
            if name.startswith("_"):
                continue
            for token in forbidden_tokens:
                if token in name:
                    violations.append(
                        f"{cls.__name__}.{name} contains forbidden token {token}"
                    )
    for path in core_files:
        body = path.read_text(encoding="utf-8")
        for token in forbidden_tokens:
            if token in body:
                violations.append(f"{path} contains forbidden token {token}")
    return sorted(violations)


def shared_directory_query_base(fixture: str) -> DirectoryQueryBase:
    decoded = json.loads(shared_fixture(fixture))
    return DirectoryQueryBase(
        caller_ura=decoded["caller_ura"],
        callee_ura=decoded["callee_ura"],
        subject_ura=decoded["subject_ura"],
        descriptor_version=decoded["descriptor_version"],
        nonce_base64=decoded["nonce_base64"],
        causal_context=decoded["causal_context"],
        cursor=decoded.get("cursor", ""),
        limit=decoded.get("limit", 0),
        metadata=decoded.get("metadata", {}),
    )


def shared_local_resource_ref_request() -> LocalResourceRefRequest:
    decoded = json.loads(shared_fixture("local-resource-ref-request.v4.json"))
    return LocalResourceRefRequest(decoded["path"], decoded["capability"])


def shared_ability_package_manifest() -> AbilityPackageManifest:
    decoded = json.loads(shared_fixture("ability-package-manifest.v4.json"))
    return AbilityPackageManifest(
        name=decoded["name"],
        namespace=decoded["namespace"],
        description=decoded["description"],
        input_schema=decoded["input_schema"],
        exec=decoded["exec"],
    )


def shared_publication_validate_package_request() -> bytes:
    return json.dumps(
        {"manifest": json.loads(shared_fixture("ability-package-manifest.v4.json"))},
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")


def shared_ability_deploy_request() -> AbilityDeployRequest:
    decoded = json.loads(shared_fixture("ability-deploy-request.v4.json"))
    return AbilityDeployRequest(
        caller_ura=decoded["caller_ura"],
        callee_ura=decoded["callee_ura"],
        subject_ura=decoded["subject_ura"],
        descriptor_version=decoded["descriptor_version"],
        nonce_base64=decoded["nonce_base64"],
        causal_context=decoded["causal_context"],
        resource_ref=ResourceRef.from_json(json.dumps(decoded["resource_ref"])),
        node_id=decoded["node_id"],
        metadata=decoded["metadata"],
    )


def shared_unpublish_ability_request() -> UnpublishAbilityRequest:
    deploy = shared_ability_deploy_request()
    return UnpublishAbilityRequest(
        caller_ura=deploy.caller_ura,
        callee_ura=deploy.callee_ura,
        subject_ura=deploy.subject_ura,
        descriptor_version=deploy.descriptor_version,
        nonce_base64=deploy.nonce_base64,
        causal_context=deploy.causal_context,
        ability_ura="easynet:///r/example/ability/device.dev-a.er.weather",
    )


def shared_mission_carrier_base(fixture: str) -> MissionCarrierBase:
    decoded = json.loads(shared_fixture(fixture))
    return MissionCarrierBase(
        caller_ura=decoded["caller_ura"],
        callee_ura=decoded["callee_ura"],
        subject_ura=decoded["subject_ura"],
        descriptor_version=decoded["descriptor_version"],
        nonce_base64=decoded["nonce_base64"],
        causal_context=decoded["causal_context"],
        metadata=decoded.get("metadata", {}),
    )


def shared_mission_run_request() -> MissionRunRequest:
    decoded = json.loads(shared_fixture("mission-run-request.v4.json"))
    return MissionRunRequest(
        base=shared_mission_carrier_base("mission-run-request.v4.json"),
        source=decoded["source"],
        label=decoded["label"],
    )


def shared_mission_run_file_request() -> MissionRunFileRequest:
    decoded = json.loads(shared_fixture("mission-run-file-request.v4.json"))
    return MissionRunFileRequest(
        base=shared_mission_carrier_base("mission-run-file-request.v4.json"),
        path=decoded["path"],
        label=decoded["label"],
    )


def shared_mission_track_request() -> MissionTrackRequest:
    decoded = json.loads(shared_fixture("mission-track-request.v4.json"))
    return MissionTrackRequest(
        base=shared_mission_carrier_base("mission-track-request.v4.json"),
        mission_id=decoded["mission_id"],
    )


def shared_mission_cancel_request() -> MissionCancelRequest:
    decoded = json.loads(shared_fixture("mission-cancel-request.v4.json"))
    return MissionCancelRequest(
        base=shared_mission_carrier_base("mission-cancel-request.v4.json"),
        mission_id=decoded["mission_id"],
    )


def shared_mission_status_without_parent_anchor() -> bytes:
    status = json.loads(shared_fixture("mission-status.v4.json"))
    status["parent_receipt_ura"] = None
    return json.dumps(status, separators=(",", ":"), sort_keys=True).encode("utf-8")


def shared_admin_carrier_base(fixture: str) -> AdminCarrierBase:
    decoded = json.loads(shared_fixture(fixture))
    return AdminCarrierBase(
        caller_ura=decoded["caller_ura"],
        callee_ura=decoded["callee_ura"],
        subject_ura=decoded["subject_ura"],
        descriptor_version=decoded["descriptor_version"],
        nonce_base64=decoded["nonce_base64"],
        causal_context=decoded["causal_context"],
        metadata=decoded.get("metadata", {}),
    )


def shared_admin_agent_list_request() -> AdminAgentListRequest:
    return AdminAgentListRequest(
        base=shared_admin_carrier_base("admin-agent-list-request.v4.json")
    )


def shared_admin_agent_start_request() -> AdminAgentStartRequest:
    decoded = json.loads(shared_fixture("admin-agent-start-request.v4.json"))
    return AdminAgentStartRequest(
        base=shared_admin_carrier_base("admin-agent-start-request.v4.json"),
        name=decoded["name"],
        agent_type=decoded.get("agent_type", ""),
        model=decoded.get("model", ""),
        label=decoded.get("label", ""),
    )


def shared_admin_agent_stop_request() -> AdminAgentStopRequest:
    decoded = json.loads(shared_fixture("admin-agent-stop-request.v4.json"))
    return AdminAgentStopRequest(
        base=shared_admin_carrier_base("admin-agent-stop-request.v4.json"),
        name=decoded.get("name", ""),
        agent_ura=decoded.get("agent_ura", ""),
    )


def shared_admin_agent_refresh_request() -> AdminAgentRefreshRequest:
    decoded = json.loads(shared_fixture("admin-agent-refresh-request.v4.json"))
    return AdminAgentRefreshRequest(
        base=shared_admin_carrier_base("admin-agent-refresh-request.v4.json"),
        name=decoded.get("name", ""),
    )


def shared_admin_session_list_request() -> AdminSessionListRequest:
    decoded = json.loads(shared_fixture("admin-session-list-request.v4.json"))
    return AdminSessionListRequest(
        base=shared_admin_carrier_base("admin-session-list-request.v4.json"),
        include_terminated=decoded.get("include_terminated"),
    )


def shared_control_only_gateway_status() -> bytes:
    status = json.loads(shared_fixture("gateway-status.v4.json"))
    status["ready"] = False
    status["state"] = "degraded"
    status["runtime_ready"] = False
    status["directory_ready"] = False
    status["metadata"]["lifecycle_state"] = "control_only"
    return json.dumps(status, separators=(",", ":"), sort_keys=True).encode("utf-8")


def shared_events_carrier_base(fixture: str) -> EventsCarrierBase:
    decoded = json.loads(shared_fixture(fixture))
    return EventsCarrierBase(
        caller_ura=decoded["caller_ura"],
        callee_ura=decoded["callee_ura"],
        subject_ura=decoded["subject_ura"],
        descriptor_version=decoded["descriptor_version"],
        nonce_base64=decoded["nonce_base64"],
        causal_context=decoded["causal_context"],
        metadata=decoded.get("metadata", {}),
    )


def shared_events_directory_subscription_request() -> EventsDirectorySubscriptionRequest:
    decoded = json.loads(shared_fixture("events-directory-subscription-request.v4.json"))
    cursor = decoded.get("resume_cursor")
    return EventsDirectorySubscriptionRequest(
        base=shared_events_carrier_base(
            "events-directory-subscription-request.v4.json"
        ),
        realm=decoded.get("realm", ""),
        agent_ura=decoded.get("agent_ura", ""),
        resume_cursor=EventCursor(cursor["stream"], cursor["sequence"]) if cursor else None,
        heartbeat_interval_ms=decoded.get("heartbeat_interval_ms", 0),
    )


def shared_events_directory_subscription_request_json() -> bytes:
    return shared_events_directory_subscription_request().to_json_bytes("directory")


def shared_events_projection_input() -> EventProjectionInput:
    frame = shared_events_frame("event.directory.v4.json")
    cursor = shared_events_cursor_from_frame(frame)
    return EventProjectionInput(
        cursor=cursor,
        event=frame["payload"],
        event_id=frame["event_id"],
        resume_token=frame["resume_token"],
        tenant_ref=frame["tenant_ref"],
    )


def shared_events_projection_input_json() -> bytes:
    return shared_events_projection_input().to_json_bytes()


def shared_events_drop_report_input() -> EventDropReportInput:
    frame = shared_events_frame("event.directory-drop-report.v4.json")
    cursor = shared_events_cursor_from_frame(frame)
    return EventDropReportInput(
        cursor=cursor,
        occurred_unix_ms=frame["occurred_unix_ms"],
        dropped_count=frame["dropped_count"],
        reconnect_after_ms=frame["reconnect_after_ms"],
        reason=frame["metadata"]["reason"],
        event_id=frame["event_id"],
        resume_token=frame["resume_token"],
        tenant_ref=frame["tenant_ref"],
    )


def shared_events_drop_report_input_json() -> bytes:
    return shared_events_drop_report_input().to_json_bytes()


def shared_events_terminal_input() -> EventTerminalInput:
    frame = shared_events_frame("event.directory-terminal.v4.json")
    cursor = shared_events_cursor_from_frame(frame)
    return EventTerminalInput(
        cursor=cursor,
        occurred_unix_ms=frame["occurred_unix_ms"],
        reconnect_after_ms=frame["reconnect_after_ms"],
        reason=frame["metadata"]["reason"],
        event_id=frame["event_id"],
        resume_token=frame["resume_token"],
        tenant_ref=frame["tenant_ref"],
    )


def shared_events_terminal_input_json() -> bytes:
    return shared_events_terminal_input().to_json_bytes()


def shared_events_frame_without_cursor_token(fixture: str) -> bytes:
    frame = shared_events_frame(fixture)
    del frame["cursor"]["token"]
    return json.dumps(frame, separators=(",", ":"), sort_keys=True).encode("utf-8")


def shared_events_drop_report_without_dropped_count() -> bytes:
    frame = shared_events_frame("event.directory-drop-report.v4.json")
    frame["dropped_count"] = 0
    return json.dumps(frame, separators=(",", ":"), sort_keys=True).encode("utf-8")


def shared_events_terminal_without_terminal_flag() -> bytes:
    frame = shared_events_frame("event.directory-terminal.v4.json")
    frame["terminal"] = False
    return json.dumps(frame, separators=(",", ":"), sort_keys=True).encode("utf-8")


def shared_events_frame(fixture: str) -> dict[str, object]:
    return json.loads(shared_fixture(fixture))


def shared_events_cursor_from_frame(frame: dict[str, object]) -> EventCursor:
    cursor = frame["cursor"]
    return EventCursor(cursor["stream"], cursor["sequence"])


def shared_surface_carrier_base(fixture: str) -> SurfaceCarrierBase:
    decoded = json.loads(shared_fixture(fixture))
    return SurfaceCarrierBase(
        caller_ura=decoded["caller_ura"],
        callee_ura=decoded["callee_ura"],
        subject_ura=decoded["subject_ura"],
        descriptor_version=decoded["descriptor_version"],
        nonce_base64=decoded["nonce_base64"],
        causal_context=decoded["causal_context"],
        metadata=decoded.get("metadata", {}),
    )


def shared_surface_list_pages_request() -> SurfaceListPagesRequest:
    decoded = json.loads(shared_fixture("surface-list-pages-request.v4.json"))
    return SurfaceListPagesRequest(
        base=shared_surface_carrier_base("surface-list-pages-request.v4.json"),
        limit=decoded.get("limit", 0),
        cursor=decoded.get("cursor", ""),
    )


def shared_surface_create_page_request() -> SurfaceCreatePageRequest:
    decoded = json.loads(shared_fixture("surface-create-page-request.v4.json"))
    return SurfaceCreatePageRequest(
        base=shared_surface_carrier_base("surface-create-page-request.v4.json"),
        project_id=decoded["project_id"],
        folder=decoded["folder"],
        visibility=decoded.get("visibility", ""),
    )


def shared_surface_delete_page_request() -> SurfaceDeletePageRequest:
    decoded = json.loads(shared_fixture("surface-delete-page-request.v4.json"))
    return SurfaceDeletePageRequest(
        base=shared_surface_carrier_base("surface-delete-page-request.v4.json"),
        project_id=decoded["project_id"],
    )


def shared_surface_manifest_request() -> SurfaceManifestRequest:
    decoded = json.loads(shared_fixture("surface-manifest-request.v4.json"))
    return SurfaceManifestRequest(
        base=shared_surface_carrier_base("surface-manifest-request.v4.json"),
        project_id=decoded["project_id"],
    )


def shared_surface_page_page_with_oversized_limit() -> bytes:
    page = json.loads(shared_fixture("surface-page-page.v4.json"))
    page["limit"] = MAX_SURFACE_PAGE_SIZE + 1
    return json.dumps(page, separators=(",", ":"), sort_keys=True).encode("utf-8")


def shared_surface_manifest_without_entrypoint() -> bytes:
    manifest = json.loads(shared_fixture("surface-manifest.v4.json"))
    del manifest["entrypoint"]
    return json.dumps(manifest, separators=(",", ":"), sort_keys=True).encode("utf-8")


def shared_compatibility_base(fixture: str) -> CompatibilityCarrierBase:
    decoded = json.loads(shared_fixture(fixture))
    return CompatibilityCarrierBase(
        caller_ura=decoded["caller_ura"],
        callee_ura=decoded["callee_ura"],
        subject_ura=decoded["subject_ura"],
        descriptor_version=decoded["descriptor_version"],
        nonce_base64=decoded["nonce_base64"],
        causal_context=decoded["causal_context"],
        auth_token=decoded.get("auth_token", ""),
        metadata=decoded.get("metadata", {}),
    )


def shared_compatibility_list_models_request() -> CompatibilityListModelsRequest:
    return CompatibilityListModelsRequest(
        shared_compatibility_base("compatibility-list-models-request.v4.json")
    )


def shared_compatibility_chat_completion_request() -> CompatibilityChatCompletionRequest:
    decoded = json.loads(shared_fixture("compatibility-chat-completion-request.v4.json"))
    return CompatibilityChatCompletionRequest(
        shared_compatibility_base("compatibility-chat-completion-request.v4.json"),
        decoded["request"],
    )


def shared_compatibility_stream_chat_completion_request() -> CompatibilityStreamChatCompletionRequest:
    decoded = json.loads(
        shared_fixture("compatibility-stream-chat-completion-request.v4.json")
    )
    return CompatibilityStreamChatCompletionRequest(
        shared_compatibility_base("compatibility-stream-chat-completion-request.v4.json"),
        decoded["request"],
    )


def shared_compatibility_file_upload_request() -> CompatibilityFileUploadRequest:
    decoded = json.loads(shared_fixture("compatibility-file-upload-request.v4.json"))
    return CompatibilityFileUploadRequest(
        id=decoded["id"],
        file_ref=decoded["file_ref"],
        owner_ura=decoded["owner_ura"],
        filename=decoded["filename"],
        purpose=decoded["purpose"],
        content_type=decoded["content_type"],
        content_hash=decoded["content_hash"],
        size_bytes=decoded["size_bytes"],
        created_at=decoded["created_at"],
        metadata=decoded.get("metadata", {}),
    )


def shared_compatibility_file_request() -> CompatibilityFileRequest:
    decoded = json.loads(shared_fixture("compatibility-file-request.v4.json"))
    return CompatibilityFileRequest(
        id=decoded["id"],
        file_ref=decoded["file_ref"],
        owner_ura=decoded["owner_ura"],
        filename=decoded["filename"],
        purpose=decoded["purpose"],
        content_type=decoded["content_type"],
        content_hash=decoded["content_hash"],
        size_bytes=decoded["size_bytes"],
        created_at=decoded["created_at"],
        metadata=decoded.get("metadata", {}),
    )


def shared_compatibility_file_delete_request() -> CompatibilityFileDeleteRequest:
    decoded = json.loads(shared_fixture("compatibility-file-delete-request.v4.json"))
    return CompatibilityFileDeleteRequest(
        id=decoded["id"],
        deleted=decoded["deleted"],
        metadata=decoded.get("metadata", {}),
    )


def shared_compatibility_stream_request_json() -> bytes:
    request = json.loads(shared_fixture("compatibility-stream-chat-completion-request.v4.json"))
    request["request"]["stream"] = True
    return json.dumps(request, separators=(",", ":"), sort_keys=True).encode("utf-8")


def shared_feature_discovery_json(abi_version: int) -> bytes:
    return json.dumps(
        {
            "abi_version": abi_version,
            "sdk_version": "0.91.30",
            "profiles": {"runtime_core": "partial"},
            "symbols": {"runtime_health": True},
            "axon_pb": False,
        },
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")


def shared_invocation_draft() -> InvocationDraft:
    return InvocationDraft.from_json(shared_fixture("invocation.complete.v4.json"))


def shared_invocation_builder() -> InvocationBuilder:
    draft = shared_invocation_draft()
    builder = (
        InvocationBuilder()
        .with_caller_ura(draft.caller_ura)
        .with_callee_ura(draft.callee_ura)
        .with_descriptor_ref(draft.descriptor_ref)
        .with_subject_ura(draft.subject_ura)
        .with_nonce_base64(draft.nonce_base64)
        .with_causal_context(draft.causal_context)
        .with_content_type(draft.content_type)
        .with_metadata(draft.metadata)
    )
    if draft._has_args:
        builder.with_json_args(draft.args)
    else:
        assert draft.arguments_base64 is not None
        builder.with_arguments_base64(draft.arguments_base64)
    if draft.caller_signature is not None:
        builder.with_caller_signature(draft.caller_signature)
    return builder


def shared_invocation_signature() -> InvocationSignature:
    return InvocationSignature(
        algorithm="ed25519",
        signature_base64="c2lnbmF0dXJl",
        key_id_hint="caller-key",
    )


def shared_receipt_fetch_request() -> ReceiptFetchRequest:
    decoded = json.loads(shared_fixture("receipt-fetch-request.v4.json"))
    return ReceiptFetchRequest(
        caller_ura=decoded["caller_ura"],
        callee_ura=decoded["callee_ura"],
        descriptor_ref=decoded["descriptor_ref"],
        subject_ura=decoded["subject_ura"],
        descriptor_version=decoded["descriptor_version"],
        nonce_base64=decoded["nonce_base64"],
        causal_context=decoded["causal_context"],
        invocation_ura=decoded.get("invocation_ura", ""),
        request_id=decoded.get("request_id", ""),
        trace_id=decoded.get("trace_id", ""),
        metadata=decoded.get("metadata", {}),
    )


def shared_control_only_health_json() -> bytes:
    health = json.loads(shared_fixture("health.ready.v4.json"))
    health["invocation_ready"] = False
    health["runtime_ready"] = False
    health["diagnostics"] = ["invocation endpoint unavailable"]
    return json.dumps(health, separators=(",", ":"), sort_keys=True).encode("utf-8")


def shared_ability_query() -> AbilityQuery:
    decoded = json.loads(shared_fixture("directory-list-abilities-request.v4.json"))
    return AbilityQuery(
        base=shared_directory_query_base("directory-list-abilities-request.v4.json"),
        scope=decoded["scope"],
        owner_ura=decoded["owner_ura"],
        ability_ura=decoded["ability_ura"],
    )


def shared_resolve_query() -> ResolveQuery:
    decoded = json.loads(shared_fixture("directory-resolve-request.v4.json"))
    return ResolveQuery(
        base=shared_directory_query_base("directory-resolve-request.v4.json"),
        query_name=decoded["query_name"],
        ability_name=decoded["ability_name"],
        qtype=decoded["qtype"],
    )


def assert_json_equivalent(actual: bytes, expected: bytes) -> None:
    if json.loads(actual.decode("utf-8")) != json.loads(expected.decode("utf-8")):
        raise AssertionError(
            "JSON mismatch\n"
            f"actual: {actual.decode('utf-8')}\n"
            f"expected: {expected.decode('utf-8')}"
        )


if __name__ == "__main__":
    unittest.main()
