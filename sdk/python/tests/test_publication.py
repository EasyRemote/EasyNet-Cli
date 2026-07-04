import json
import tempfile
import unittest
from pathlib import Path

from easynet_sdk import (
    AbilityDeployRequest,
    AbilityImplID,
    EasyRemotePublicationAdapter,
    ErrorCode,
    LocalResourceRefRequest,
    PublicationClient,
    PublishedAbilityQuery,
    PublishedAbilityShowRequest,
    ResourceRef,
    RuntimeClient,
    RuntimePublicationTransport,
    SDKError,
    UnpublishAbilityRequest,
    is_code,
)

from test_runtime import MemoryRuntimeTransport


RESOURCE_REF_JSON = b"""{
  "resource_ura": "easynet:///r/example/resource/device.dev-a/fs/tmp/easynet-weather-package",
  "owner_ura": "easynet:///r/example/device/dev-a",
  "namespace": "fs",
  "display_path": "tmp/easynet-weather-package",
  "capability": "read",
  "expires_unix_ms": 4102444800000,
  "revision": "fs-local-mapping-v1"
}"""

PACKAGE_VALIDATION_JSON = b"""{
  "profile": "publication",
  "kind": "package_validation",
  "valid": true,
  "package_path": "/tmp/easynet-weather-package",
  "manifest_path": "/tmp/easynet-weather-package/ability.json",
  "manifest_hash": "sha256:09c6bb09967428f12db1c5f0d0ae726c448dabf01bf7cea8476f4eabdf613bd1",
  "manifest": {
    "name": "weather",
    "namespace": "er",
    "wire_key": "er.weather",
    "descriptor_version": "1.0.0",
    "description": "Weather stream",
    "exec_kind": "host_stream",
    "timeout_seconds": null,
    "input_schema": {"type": "object", "properties": {}},
    "output_schema": null
  },
  "errors": [],
  "metadata": {"profile": "publication", "frame_contract_owner": "daemon_sdk"}
}"""

DEPLOY_INVOCATION_JSON = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {
    "resource_ref": {
      "resource_ura": "easynet:///r/example/resource/device.dev-a/fs/tmp/easynet-weather-package",
      "owner_ura": "easynet:///r/example/device/dev-a",
      "namespace": "fs",
      "display_path": "tmp/easynet-weather-package",
      "capability": "read",
      "expires_unix_ms": 4102444800000,
      "revision": "fs-local-mapping-v1"
    },
    "node_id": "local"
  },
  "content_type": "application/json",
  "metadata": {
    "request_id": "publication-deploy-1",
    "profile": "publication",
    "system_ability": "ability.deploy",
    "carrier_owner": "daemon_sdk"
  }
}"""

LIST_INVOCATION_JSON = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.meta.list_abilities@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"agent_ura": "easynet:///r/example/device/dev-a"},
  "content_type": "application/json",
  "metadata": {
    "profile": "publication",
    "system_ability": "meta.list_abilities",
    "carrier_owner": "daemon_sdk"
  }
}"""

SHOW_INVOCATION_JSON = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.meta.list_abilities@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {
    "subject_ura": "easynet:///r/example/ability/device.dev-a.er.weather"
  },
  "content_type": "application/json",
  "metadata": {
    "profile": "publication",
    "system_ability": "meta.list_abilities",
    "carrier_owner": "daemon_sdk"
  }
}"""

UNPUBLISH_INVOCATION_JSON = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather"},
  "content_type": "application/json",
  "metadata": {
    "profile": "publication",
    "system_ability": "ability.unpublish",
    "carrier_owner": "daemon_sdk"
  }
}"""


class MemoryPublicationTransport:
    def __init__(self) -> None:
        self.resource_ref_json = RESOURCE_REF_JSON
        self.package_validation_json = PACKAGE_VALIDATION_JSON
        self.deploy_result_json = (
            b'{"profile":"publication","kind":"ability_deploy_result",'
            b'"public_name":"weather","namespace":"er",'
            b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"node_id":"local","install_id":"install-1","state":"enabled",'
            b'"mutated_by":"","bundle":"","metadata":{}}'
        )
        self.deploy_invocation_json = DEPLOY_INVOCATION_JSON
        self.plugin_install_json = (
            b'{"profile":"publication","kind":"plugin_install",'
            b'"source":"file:///tmp/plugin","install_id":"install-1",'
            b'"status":"installed","metadata":{}}'
        )
        self.list_json = (
            b'{"profile":"publication","kind":"published_ability_page",'
            b'"item_kind":"published_ability","items":[{"descriptor":'
            b'{"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",'
            b'"descriptor_version":"1.0.0","schema_hash":"sha256:abc",'
            b'"owner_ura":"easynet:///r/example/device/dev-a"},'
            b'"implementation":{"impl_id":"impl-1","impl_hash":"sha256:def",'
            b'"runtime_env":"python","enabled":true},"metadata":{}}],'
            b'"next_cursor":null,"limit":50,"source":"read_model","metadata":{}}'
        )
        self.show_json = (
            b'{"descriptor":{"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",'
            b'"descriptor_version":"1.0.0","schema_hash":"sha256:abc",'
            b'"owner_ura":"easynet:///r/example/device/dev-a"},'
            b'"implementation":{"impl_id":"impl-1","impl_hash":"sha256:def",'
            b'"runtime_env":"python","enabled":true},"metadata":{}}'
        )
        self.show_invocation_json = SHOW_INVOCATION_JSON
        self.enable_json = b'{"profile":"publication","kind":"ability_impl_enabled","metadata":{}}'
        self.disable_json = b'{"profile":"publication","kind":"ability_impl_disabled","metadata":{}}'
        self.unpublish_invocation_json = UNPUBLISH_INVOCATION_JSON
        self.unpublish_json = (
            b'{"profile":"publication","kind":"ability_unpublished",'
            b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",'
            b'"metadata":{}}'
        )
        self.seen_request: dict[str, object] | None = None
        self.seen_projection: dict[str, object] | None = None
        self.close_calls = 0

    def _remember(self, request_json: bytes) -> None:
        self.seen_request = json.loads(request_json.decode("utf-8"))

    def build_resource_ref(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.resource_ref_json

    def validate_package(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.package_validation_json

    def deploy_ability(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.deploy_result_json

    def build_deploy_invocation(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.deploy_invocation_json

    def project_deploy_result(self, result_json: bytes) -> bytes:
        self.seen_projection = json.loads(result_json.decode("utf-8"))
        return self.deploy_result_json

    def install_plugin(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.plugin_install_json

    def list_abilities(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.list_json

    def build_list_abilities_invocation(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return LIST_INVOCATION_JSON

    def project_ability_page(self, page_json: bytes) -> bytes:
        self.seen_projection = json.loads(page_json.decode("utf-8"))
        return self.list_json

    def build_show_ability_invocation(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.show_invocation_json

    def project_ability_record(self, record_json: bytes) -> bytes:
        self.seen_projection = json.loads(record_json.decode("utf-8"))
        return self.show_json

    def project_unpublish_result(self, result_json: bytes) -> bytes:
        self.seen_projection = json.loads(result_json.decode("utf-8"))
        return self.unpublish_json

    def show_ability(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.show_json

    def enable_ability_impl(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.enable_json

    def disable_ability_impl(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.disable_json

    def build_unpublish_invocation(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.unpublish_invocation_json

    def unpublish_ability(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.unpublish_json

    def close(self) -> None:
        self.close_calls += 1


class _EasyRemoteIdentity:
    realm = "example"
    node_id = "dev-a"
    device_ura = "easynet:///r/example/device/dev-a"


class _EasyRemoteDevice:
    def __init__(self, node_id: str) -> None:
        self.owner_ura = f"easynet:///r/example/device/{node_id}"


class _EasyRemoteInvocation:
    def __init__(self, response: dict[str, object]) -> None:
        self._response = response

    def result(self) -> dict[str, object]:
        return self._response


class _EasyRemoteClient:
    def __init__(self, *responses: dict[str, object]) -> None:
        self.responses = list(responses)
        self.invocations: list[dict[str, object]] = []

    def _who(self) -> _EasyRemoteIdentity:
        return _EasyRemoteIdentity()

    def target(self, ability: str, **kwargs: object) -> dict[str, object]:
        return {"ability": ability, **kwargs}

    def device(self, node_id: str) -> _EasyRemoteDevice:
        return _EasyRemoteDevice(node_id)

    def invoke(self, target: object, **kwargs: object) -> _EasyRemoteInvocation:
        self.invocations.append({"target": target, "args": dict(kwargs)})
        return _EasyRemoteInvocation(self.responses.pop(0))


class _EasyRemoteUraProjection:
    def __init__(self, kind: str) -> None:
        self.kind = kind


class _EasyRemoteAddressing:
    def parse_ura(self, value: str) -> _EasyRemoteUraProjection:
        if "/ability/" in value:
            return _EasyRemoteUraProjection("ability")
        if "/device/" in value:
            return _EasyRemoteUraProjection("device")
        if "/agent/" in value:
            return _EasyRemoteUraProjection("agent")
        if "/hub" in value:
            return _EasyRemoteUraProjection("hub")
        if "/user/" in value:
            return _EasyRemoteUraProjection("user")
        raise ValueError(f"invalid URA {value!r}")

    def resource_ura(self, realm: str, owner_id: str, path: str) -> str:
        return f"easynet:///r/{realm}/resource/{owner_id}/{path.strip('/')}"


class DeployRuntimeTransport(MemoryRuntimeTransport):
    def __init__(self, output_json: object | None = None) -> None:
        super().__init__()
        self.output_json = (
            {
                "public_name": "weather",
                "namespace": "er",
                "ability_ura": (
                    "easynet:///r/example/ability/device.dev-a.er.weather"
                ),
                "node_id": "local",
                "install_id": "install-1",
                "state": "enabled",
            }
            if output_json is None
            else output_json
        )

    def invoke(self, draft_json: bytes) -> bytes:
        self.seen_draft = json.loads(draft_json.decode("utf-8"))
        return json.dumps(
            {
                "ok": True,
                "tuple": self.seen_draft,
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_base64": "e30=",
                "output_json": self.output_json,
                "elapsed_ms": 7,
                "receipt": None,
                "error": None,
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")


def resource_ref() -> ResourceRef:
    return ResourceRef.from_json(RESOURCE_REF_JSON)


def deploy_request() -> AbilityDeployRequest:
    return AbilityDeployRequest(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/device/dev-a",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        resource_ref=resource_ref(),
        node_id="local",
        metadata={"request_id": "publication-deploy-1"},
    )


def ability_query() -> PublishedAbilityQuery:
    return PublishedAbilityQuery(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/device/dev-a",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
    )


def show_request() -> PublishedAbilityShowRequest:
    return PublishedAbilityShowRequest(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/device/dev-a",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        descriptor_ref="easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
    )


def unpublish_request() -> UnpublishAbilityRequest:
    return UnpublishAbilityRequest(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/device/dev-a",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        ability_ura="easynet:///r/example/ability/device.dev-a.er.weather",
    )


class PublicationTests(unittest.TestCase):
    def test_easyremote_adapter_installs_with_sdk_resource_ref(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            package = Path(tmp) / "pkg"
            package.mkdir()
            client = _EasyRemoteClient(
                {
                    "install_id": "inst-1",
                    "ability_ura": "easynet:///r/example/ability/device.dev-a.er.fn",
                    "state": "ACTIVE",
                }
            )

            result = EasyRemotePublicationAdapter(
                client,
                addressing=_EasyRemoteAddressing(),
            ).install_ability(package)

        self.assertEqual(result.install_id, "inst-1")
        invocation = client.invocations[0]
        self.assertEqual(invocation["target"]["ability"], "ability.deploy")
        ref = invocation["args"]["resource_ref"]
        self.assertEqual(ref["namespace"], "fs")
        self.assertEqual(ref["owner_ura"], "easynet:///r/example/device/dev-a")
        self.assertEqual(ref["capability"], "read")
        self.assertEqual(ref["revision"], "fs-local-mapping-v1")
        self.assertEqual(invocation["target"]["subject"], ref["resource_ura"])
        self.assertEqual(invocation["args"]["node_id"], "local")

    def test_easyremote_adapter_lists_against_remote_device_owner(self) -> None:
        row = {
            "name": "er.fn",
            "ability_ura": "easynet:///r/example/ability/device.gpu-2.er.fn",
            "owner_ura": "easynet:///r/example/device/gpu-2",
            "state": "ACTIVE",
        }
        client = _EasyRemoteClient({"abilities": [row]})

        records = EasyRemotePublicationAdapter(
            client,
            addressing=_EasyRemoteAddressing(),
        ).list_abilities(
            node="gpu-2",
            owner_ura="easynet:///r/example/device/gpu-2",
            scope="realm",
        )

        self.assertEqual(records[0].ability_ura, row["ability_ura"])
        invocation = client.invocations[0]
        self.assertEqual(invocation["target"]["ability"], "meta.list_abilities")
        self.assertEqual(
            invocation["target"]["owner_ura"], "easynet:///r/example/device/gpu-2"
        )
        self.assertEqual(
            invocation["args"],
            {
                "agent_ura": "easynet:///r/example/device/gpu-2",
                "scope": "realm",
            },
        )

    def test_build_resource_ref_and_validate_package(self) -> None:
        transport = MemoryPublicationTransport()
        client = PublicationClient(transport)

        ref = client.build_local_resource_ref(
            LocalResourceRefRequest(path="/tmp/easynet-weather-package", capability="read")
        )
        self.assertTrue(ref.resource_ura)

        validation = client.validate_package("/tmp/easynet-weather-package")
        self.assertTrue(validation.valid)
        self.assertEqual(validation.manifest.wire_key, "er.weather")
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["path"], "/tmp/easynet-weather-package")

    def test_deploy_and_build_deploy_invocation(self) -> None:
        transport = MemoryPublicationTransport()
        client = PublicationClient(transport)

        result = client.deploy_ability(deploy_request())
        self.assertEqual(result.state, "enabled")
        self.assertEqual(result.kind, "ability_deploy_result")

        draft = client.build_deploy_invocation(deploy_request())
        self.assertEqual(
            draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0",
        )
        self.assertEqual(draft.args["node_id"], "local")

    def test_runtime_publication_transport_executes_deploy_via_runtime_core(self) -> None:
        carrier = MemoryPublicationTransport()
        runtime_transport = DeployRuntimeTransport()
        client = PublicationClient(
            RuntimePublicationTransport(
                carrier=carrier,
                runtime=RuntimeClient(runtime_transport),
            )
        )

        result = client.deploy_ability(deploy_request())

        self.assertEqual(result.state, "enabled")
        assert carrier.seen_request is not None
        self.assertEqual(carrier.seen_request["node_id"], "local")
        assert runtime_transport.seen_draft is not None
        self.assertEqual(
            runtime_transport.seen_draft["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0",
        )
        self.assertEqual(
            runtime_transport.seen_draft["metadata"]["system_ability"],
            "ability.deploy",
        )
        assert carrier.seen_projection is not None
        self.assertEqual(carrier.seen_projection["state"], "enabled")

        client.close()
        client.close()
        self.assertEqual(runtime_transport.close_calls, 1)
        self.assertEqual(carrier.close_calls, 1)

    def test_runtime_publication_transport_rejects_invalid_deploy_output(self) -> None:
        client = PublicationClient(
            RuntimePublicationTransport(
                carrier=MemoryPublicationTransport(),
                runtime=RuntimeClient(DeployRuntimeTransport(output_json=[])),
            )
        )

        with self.assertRaises(SDKError) as raised:
            client.deploy_ability(deploy_request())

        self.assertTrue(is_code(raised.exception, ErrorCode.INVALID_ARGUMENT))

    def test_runtime_publication_transport_executes_list_via_runtime_core(self) -> None:
        carrier = MemoryPublicationTransport()
        runtime_transport = DeployRuntimeTransport(
            output_json={
                "abilities": [
                    {
                        "name": "weather",
                        "ability_ura": (
                            "easynet:///r/example/ability/device.dev-a.er.weather"
                        ),
                        "owner_ura": "easynet:///r/example/device/dev-a",
                        "version": "1.0.0",
                        "schema_hash": "sha256:abc",
                    }
                ]
            }
        )
        client = PublicationClient(
            RuntimePublicationTransport(
                carrier=carrier,
                runtime=RuntimeClient(runtime_transport),
            )
        )

        page = client.list_abilities(ability_query())

        self.assertEqual(page.limit, 50)
        self.assertEqual(len(page.items), 1)
        assert runtime_transport.seen_draft is not None
        self.assertEqual(
            runtime_transport.seen_draft["metadata"]["system_ability"],
            "meta.list_abilities",
        )
        assert carrier.seen_projection is not None
        self.assertEqual(carrier.seen_projection["limit"], 50)
        self.assertIn("abilities", carrier.seen_projection["result"])

    def test_runtime_publication_transport_executes_unpublish_via_runtime_core(self) -> None:
        carrier = MemoryPublicationTransport()
        runtime_transport = DeployRuntimeTransport(
            output_json={
                "ok": True,
                "owner_ura": "easynet:///r/example/device/dev-a",
                "public_name": "weather",
                "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                "removed_path": "/tmp/easynet/abilities/weather.ability.json",
                "content_hash": "sha256:abc",
            }
        )
        client = PublicationClient(
            RuntimePublicationTransport(
                carrier=carrier,
                runtime=RuntimeClient(runtime_transport),
            )
        )

        client.unpublish_ability(unpublish_request())

        assert runtime_transport.seen_draft is not None
        self.assertEqual(
            runtime_transport.seen_draft["metadata"]["system_ability"],
            "ability.unpublish",
        )
        assert carrier.seen_projection is not None
        self.assertEqual(carrier.seen_projection["descriptor_version"], "1.0.0")
        self.assertEqual(
            carrier.seen_projection["ability_ura"],
            "easynet:///r/example/ability/device.dev-a.er.weather",
        )
        self.assertEqual(
            carrier.seen_projection["result"]["content_hash"],
            "sha256:abc",
        )

    def test_runtime_publication_transport_executes_show_via_runtime_core(self) -> None:
        carrier = MemoryPublicationTransport()
        runtime_transport = DeployRuntimeTransport(
            output_json={
                "abilities": [
                    {
                        "name": "weather",
                        "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                        "owner_ura": "easynet:///r/example/device/dev-a",
                        "version": "1.0.0",
                    }
                ]
            }
        )
        client = PublicationClient(
            RuntimePublicationTransport(
                carrier=carrier,
                runtime=RuntimeClient(runtime_transport),
            )
        )

        ability = client.show_ability(show_request())

        self.assertEqual(
            ability.descriptor["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
        )
        assert runtime_transport.seen_draft is not None
        self.assertEqual(
            runtime_transport.seen_draft["metadata"]["system_ability"],
            "meta.list_abilities",
        )
        assert carrier.seen_projection is not None
        self.assertEqual(
            carrier.seen_projection["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
        )
        self.assertIn("abilities", carrier.seen_projection["result"])

    def test_rejects_incomplete_deploy_carrier(self) -> None:
        client = PublicationClient(MemoryPublicationTransport())
        request = deploy_request()
        with self.assertRaises(SDKError):
            client.build_deploy_invocation(
                AbilityDeployRequest(
                    caller_ura=request.caller_ura,
                    callee_ura=request.callee_ura,
                    subject_ura="",
                    descriptor_version=request.descriptor_version,
                    nonce_base64=request.nonce_base64,
                    causal_context=request.causal_context,
                    resource_ref=request.resource_ref,
                    node_id=request.node_id,
                )
            )

    def test_list_show_enable_disable_and_unpublish(self) -> None:
        transport = MemoryPublicationTransport()
        client = PublicationClient(transport)

        page = client.list_abilities(ability_query())
        self.assertEqual(page.limit, 50)
        self.assertEqual(len(page.items), 1)
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["limit"], 50)

        ability = client.show_ability(
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0"
        )
        self.assertEqual(ability.descriptor["descriptor_version"], "1.0.0")

        impl = AbilityImplID(
            impl_id="impl-1",
            ability_ura="easynet:///r/example/ability/device.dev-a.er.weather",
        )
        client.enable_ability_impl(impl)
        client.disable_ability_impl(impl)
        client.unpublish_ability(
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0"
        )

    def test_build_unpublish_invocation(self) -> None:
        client = PublicationClient(MemoryPublicationTransport())

        draft = client.build_unpublish_invocation(unpublish_request())
        self.assertEqual(
            draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0",
        )
        self.assertEqual(
            draft.args["ability_ura"],
            "easynet:///r/example/ability/device.dev-a.er.weather",
        )

        request = unpublish_request()
        with self.assertRaises(SDKError):
            client.build_unpublish_invocation(
                UnpublishAbilityRequest(
                    caller_ura=request.caller_ura,
                    callee_ura=request.callee_ura,
                    subject_ura=request.subject_ura,
                    descriptor_version=request.descriptor_version,
                    nonce_base64=request.nonce_base64,
                    causal_context=request.causal_context,
                    ability_ura="",
                )
            )

    def test_build_show_invocation(self) -> None:
        client = PublicationClient(MemoryPublicationTransport())

        draft = client.build_show_ability_invocation(show_request())
        self.assertEqual(
            draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.meta.list_abilities@1.0.0",
        )
        self.assertEqual(
            draft.args["subject_ura"],
            "easynet:///r/example/ability/device.dev-a.er.weather",
        )

        request = show_request()
        with self.assertRaises(SDKError):
            client.build_show_ability_invocation(
                PublishedAbilityShowRequest(
                    caller_ura=request.caller_ura,
                    callee_ura=request.callee_ura,
                    subject_ura=request.subject_ura,
                    descriptor_version=request.descriptor_version,
                    nonce_base64=request.nonce_base64,
                    causal_context=request.causal_context,
                    descriptor_ref="",
                )
            )

    def test_close_delegates_once_and_fails_closed(self) -> None:
        transport = MemoryPublicationTransport()
        client = PublicationClient(transport)

        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            client.build_local_resource_ref(
                LocalResourceRefRequest(
                    path="/tmp/easynet-weather-package",
                    capability="read",
                )
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(transport.seen_request)


if __name__ == "__main__":
    unittest.main()
