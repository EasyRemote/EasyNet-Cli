import json
import pathlib
import unittest

from easynet_sdk import (
    AbilityQuery,
    AbilityDeployRequest,
    AbilityPackageManifest,
    AgentQuery,
    DescriptorRefRequest,
    DeviceQuery,
    DirectoryClient,
    DirectoryQueryBase,
    ErrorCode,
    HOST_STREAM_FRAME_SCHEMA,
    HOST_STREAM_HASH_ALGORITHM,
    HostBindingClient,
    HostStreamBindingRequest,
    HostStreamEnvelope,
    HostStreamEnvelopeRequest,
    HostStreamHashState,
    HostStreamTerminalSummary,
    IdentityClient,
    IdentityProjection,
    IdentityProjectionRequest,
    InvocationDraft,
    LocalResourceRefRequest,
    MAX_DIRECTORY_PAGE_SIZE,
    PreparedInvocation,
    PublicationClient,
    ReceiptSummary,
    ReceiptVerification,
    ResourceRef,
    ResolveQuery,
    RuntimeHealth,
    RetryHint,
    SDKError,
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

    def project_descriptor_ref(self, request_json: bytes) -> bytes:
        request = json.loads(request_json.decode("utf-8"))
        if (
            request.get("descriptor_ref")
            != "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
        ):
            raise AssertionError(f"unexpected descriptor projection request: {request}")
        return self.descriptor_json

    def project_identity(self, request_json: bytes) -> bytes:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="test",
            retry=RetryHint.NEVER,
            message="not used by shared identity conformance fixture test",
        )

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
