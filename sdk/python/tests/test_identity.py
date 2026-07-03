import json
import unittest

from easynet_sdk import (
    DescriptorRefRequest,
    IdentityClient,
    LocalResourceRefRequest,
    SDKError,
)


class MemoryIdentityTransport:
    def __init__(self) -> None:
        self.descriptor_json = b"{}"
        self.identity_json = b"{}"
        self.resource_json = b"{}"
        self.seen_request: dict[str, object] | None = None

    def project_descriptor_ref(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.descriptor_json

    def project_identity(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.identity_json

    def build_resource_ref(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.resource_json


class IdentityTests(unittest.TestCase):
    def test_project_descriptor_ref_delegates_to_transport(self) -> None:
        transport = MemoryIdentityTransport()
        transport.descriptor_json = (
            b'{"kind":"descriptor_ref","valid":true,'
            b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",'
            b'"ability_ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
            b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        client = IdentityClient(transport)

        projection = client.project_descriptor_ref(
            DescriptorRefRequest(
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            )
        )

        self.assertTrue(projection.valid)
        self.assertEqual(
            projection.ability_ura,
            "easynet:///r/example/ability/device.dev-a.observe.health",
        )
        assert transport.seen_request is not None
        self.assertEqual(
            transport.seen_request["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )

    def test_build_resource_ref_validates_projection(self) -> None:
        transport = MemoryIdentityTransport()
        transport.resource_json = (
            b'{"resource_ura":"easynet:///r/example/resource/device.dev-a/fs/tmp/easynet-weather-package",'
            b'"owner_ura":"easynet:///r/example/device/dev-a","namespace":"fs",'
            b'"display_path":"tmp/easynet-weather-package","capability":"read",'
            b'"expires_unix_ms":4102444800000,"revision":"fs-local-mapping-v1"}'
        )
        client = IdentityClient(transport)

        ref = client.build_resource_ref(
            LocalResourceRefRequest(
                path="/tmp/easynet-weather-package",
                capability="read",
            )
        )

        self.assertTrue(ref.resource_ura)
        self.assertEqual(ref.revision, "fs-local-mapping-v1")
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["path"], "/tmp/easynet-weather-package")

    def test_rejects_malformed_descriptor_projection(self) -> None:
        transport = MemoryIdentityTransport()
        transport.descriptor_json = (
            b'{"kind":"descriptor_ref","valid":true,"profile":"easynet-strict-v2",'
            b'"components":{},"metadata":{}}'
        )
        client = IdentityClient(transport)

        with self.assertRaises(SDKError):
            client.project_descriptor_ref(DescriptorRefRequest("opaque"))


if __name__ == "__main__":
    unittest.main()
