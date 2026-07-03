import json
import unittest

from easynet_sdk import (
    AbilityDeployRequest,
    AbilityImplID,
    LocalResourceRefRequest,
    PublicationClient,
    PublishedAbilityQuery,
    ResourceRef,
    SDKError,
    UnpublishAbilityRequest,
)


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
            b'{"public_name":"weather","namespace":"er",'
            b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
            b'"node_id":"local","install_id":"install-1","state":"enabled"}'
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
        self.enable_json = b'{"profile":"publication","kind":"ability_impl_enabled","metadata":{}}'
        self.disable_json = b'{"profile":"publication","kind":"ability_impl_disabled","metadata":{}}'
        self.unpublish_invocation_json = UNPUBLISH_INVOCATION_JSON
        self.unpublish_json = (
            b'{"profile":"publication","kind":"ability_unpublished",'
            b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",'
            b'"metadata":{}}'
        )
        self.seen_request: dict[str, object] | None = None

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

    def install_plugin(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.plugin_install_json

    def list_abilities(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.list_json

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
        self.assertEqual(transport.seen_request["package_path"], "/tmp/easynet-weather-package")

    def test_deploy_and_build_deploy_invocation(self) -> None:
        transport = MemoryPublicationTransport()
        client = PublicationClient(transport)

        result = client.deploy_ability(deploy_request())
        self.assertEqual(result.state, "enabled")

        draft = client.build_deploy_invocation(deploy_request())
        self.assertEqual(
            draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0",
        )
        self.assertEqual(draft.args["node_id"], "local")

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


if __name__ == "__main__":
    unittest.main()
