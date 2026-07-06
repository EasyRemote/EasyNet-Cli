import json
import unittest

from easynet_sdk import (
    AddressingClient,
    DEFAULT_SIGNING_KEY_PAGE_SIZE,
    ErrorCode,
    MAX_SIGNING_KEY_PAGE_SIZE,
    DescriptorRefRequest,
    IdentityClient,
    IdentityProjectionRequest,
    LocalResourceRefRequest,
    SDKError,
    SignerHandle,
    SignerRequest,
    SigningKeyListRequest,
    SigningKeyRegistrationRequest,
    SigningKeyRevokeRequest,
    StaticSignatureProvider,
    InvocationSignature,
    is_code,
)


class MemoryIdentityTransport:
    def __init__(self) -> None:
        self.descriptor_json = b"{}"
        self.identity_json = b"{}"
        self.identity_jsons: list[bytes] = []
        self.resource_json = b"{}"
        self.invocation_json = (
            b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
            b'"callee_ura":"easynet:///r/example/device/dev-a",'
            b'"descriptor_ref":"easynet:///r/example/ability/'
            b'device.dev-a.identity.register_pubkey@1.0.0",'
            b'"subject_ura":"easynet:///r/example/user/alice",'
            b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
            b'"causal_context":{"form":"none"},'
            b'"args":{},'
            b'"content_type":"application/json",'
            b'"metadata":{"profile":"directory_identity",'
            b'"system_ability":"identity.register_pubkey"}}'
        )
        self.key_json = SIGNING_KEY_RECORD
        self.key_page_json = SIGNING_KEY_PAGE
        self.revoke_json = SIGNING_KEY_REVOKE
        self.signer_json = SIGNER_HANDLE
        self.seen_request: dict[str, object] | None = None
        self.seen_requests: list[dict[str, object]] = []
        self.close_calls = 0

    def project_descriptor_ref(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        self.seen_requests.append(self.seen_request)
        return self.descriptor_json

    def build_descriptor_ref(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        self.seen_requests.append(self.seen_request)
        return self.descriptor_json

    def project_identity(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        self.seen_requests.append(self.seen_request)
        return self._identity_json()

    def build_ura(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        self.seen_requests.append(self.seen_request)
        return self._identity_json()

    def build_resource_ref(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        self.seen_requests.append(self.seen_request)
        return self.resource_json

    def build_register_signing_key_invocation(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        self.seen_requests.append(self.seen_request)
        return self.invocation_json

    def build_list_signing_keys_invocation(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        self.seen_requests.append(self.seen_request)
        return self.invocation_json

    def build_revoke_signing_key_invocation(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        self.seen_requests.append(self.seen_request)
        return self.invocation_json

    def register_signing_key(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        self.seen_requests.append(self.seen_request)
        return self.key_json

    def list_signing_keys(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        self.seen_requests.append(self.seen_request)
        return self.key_page_json

    def revoke_signing_key(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        self.seen_requests.append(self.seen_request)
        return self.revoke_json

    def signer(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        self.seen_requests.append(self.seen_request)
        return self.signer_json

    def close(self) -> None:
        self.close_calls += 1

    def _identity_json(self) -> bytes:
        if self.identity_jsons:
            return self.identity_jsons.pop(0)
        return self.identity_json


class IdentityTests(unittest.TestCase):
    def test_descriptor_bound_resource_subject_ura_delegates_to_identity_transport(
        self,
    ) -> None:
        transport = MemoryIdentityTransport()
        transport.identity_json = (
            b'{"kind":"resource","valid":true,'
            b'"ura":"easynet:///r/example/resource/user.alice/invoke/files.read",'
            b'"profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/user/alice"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        client = AddressingClient(transport)

        got = client.descriptor_bound_resource_subject_ura(
            "easynet:///r/example/user/alice",
            "invoke/files.read",
        )
        self.assertEqual(
            got,
            "easynet:///r/example/resource/user.alice/invoke/files.read",
        )
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["kind"], "resource")
        self.assertEqual(
            transport.seen_request["owner_ura"],
            "easynet:///r/example/user/alice",
        )
        self.assertEqual(transport.seen_request["path"], "invoke/files.read")

    def test_addressing_client_is_standalone_delegated_facade(self) -> None:
        transport = MemoryIdentityTransport()
        ability_projection = (
            b'{"kind":"ability","valid":true,'
            b'"ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
            b'"profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a",'
            b'"owner_kind":"device","public_name":"observe.health",'
            b'"local_registry_ability":"easynet:///r/example/device/dev-a:observe.health",'
            b'"namespace":"observe","local_name":"health"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        descriptor_projection = (
            b'{"kind":"descriptor_ref","valid":true,'
            b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",'
            b'"ability_ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
            b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        transport.identity_json = ability_projection
        transport.descriptor_json = descriptor_projection
        client = AddressingClient(transport)

        ability_ura = client.owner_ability_ura(
            "easynet:///r/example/device/dev-a", "observe.health"
        )
        self.assertEqual(
            client.owner_ura_for_ability(ability_ura),
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(
            client.canonical_ability_descriptor_ref(ability_ura, "1.0.0"),
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(
            client.ability_ura_from_descriptor_ref(
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            ),
            "easynet:///r/example/ability/device.dev-a.observe.health",
        )
        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError):
            client.parse_ura(ability_ura)

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

    def test_project_identity_requires_ura_projection_boundary(self) -> None:
        transport = MemoryIdentityTransport()
        client = IdentityClient(transport)

        with self.assertRaises(SDKError) as missing_ura:
            client.project_identity(IdentityProjectionRequest(kind="ability"))
        self.assertEqual(missing_ura.exception.code, ErrorCode.INVALID_ARGUMENT)
        self.assertIsNone(transport.seen_request)

        with self.assertRaises(SDKError) as kind_selector:
            client.project_identity(
                IdentityProjectionRequest(
                    ura="easynet:///r/example/ability/device.dev-a.observe.health",
                    kind="ability",
                )
            )
        self.assertEqual(kind_selector.exception.code, ErrorCode.INVALID_ARGUMENT)
        self.assertIsNone(transport.seen_request)

    def test_addressing_helpers_delegate_to_identity_transport(self) -> None:
        transport = MemoryIdentityTransport()
        ability_projection = (
            b'{"kind":"ability","valid":true,'
            b'"ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
            b'"profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a",'
            b'"owner_kind":"device","public_name":"observe.health",'
            b'"local_registry_ability":"easynet:///r/example/device/dev-a:observe.health",'
            b'"namespace":"observe","local_name":"health"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        descriptor_projection = (
            b'{"kind":"descriptor_ref","valid":true,'
            b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",'
            b'"ability_ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
            b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        transport.identity_json = ability_projection
        transport.descriptor_json = descriptor_projection
        client = IdentityClient(transport)

        parsed = client.parse_ura(
            "easynet:///r/example/ability/device.dev-a.observe.health"
        )
        ability_ura = client.owner_ability_ura(
            "easynet:///r/example/device/dev-a", "observe.health"
        )
        owner_ura = client.owner_ura_for_ability(ability_ura)
        owner_ref = client.owner_ability_descriptor_ref(
            "easynet:///r/example/device/dev-a",
            "observe.health",
            "1.0.0",
        )
        built_ref = client.canonical_ability_descriptor_ref(ability_ura, "1.0.0")
        projected_ref = client.canonical_ability_descriptor_ref(built_ref)
        ability_from_ref = client.ability_ura_from_descriptor_ref(built_ref)
        address = client.ability_address(ability_ura)

        self.assertEqual(parsed.kind, "ability")
        self.assertEqual(
            ability_ura, "easynet:///r/example/ability/device.dev-a.observe.health"
        )
        self.assertEqual(owner_ura, "easynet:///r/example/device/dev-a")
        self.assertEqual(
            built_ref,
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(owner_ref, built_ref)
        self.assertEqual(projected_ref, built_ref)
        self.assertEqual(ability_from_ref, ability_ura)
        self.assertEqual(address.ability_ura, ability_ura)
        self.assertEqual(address.subject_ura, ability_ura)
        self.assertEqual(address.owner_ura, "easynet:///r/example/device/dev-a")
        self.assertEqual(address.owner_kind, "device")
        self.assertEqual(address.public_name, "observe.health")
        self.assertEqual(address.namespace, "observe")
        self.assertEqual(address.local_name, "health")
        self.assertEqual(
            transport.seen_requests,
            [
                {"ura": "easynet:///r/example/ability/device.dev-a.observe.health"},
                {
                    "kind": "ability",
                    "owner_ura": "easynet:///r/example/device/dev-a",
                    "ability_name": "observe.health",
                },
                {"ura": "easynet:///r/example/ability/device.dev-a.observe.health"},
                {
                    "kind": "ability",
                    "owner_ura": "easynet:///r/example/device/dev-a",
                    "ability_name": "observe.health",
                },
                {
                    "ability_ura": "easynet:///r/example/ability/device.dev-a.observe.health",
                    "descriptor_version": "1.0.0",
                },
                {
                    "ability_ura": "easynet:///r/example/ability/device.dev-a.observe.health",
                    "descriptor_version": "1.0.0",
                },
                {
                    "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
                },
                {
                    "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
                },
                {"ura": "easynet:///r/example/ability/device.dev-a.observe.health"},
            ],
        )

    def test_identity_provides_host_binding_descriptor_canonicalizer(self) -> None:
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

        canonicalize = client.host_binding_descriptor_ref_canonicalizer()
        canonical = canonicalize(
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
        )

        self.assertEqual(
            canonical,
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(
            transport.seen_request,
            {
                "descriptor_ref": (
                    "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
                )
            },
        )

    def test_addressing_ura_builders_delegate_to_identity_transport(self) -> None:
        transport = MemoryIdentityTransport()
        client = AddressingClient(transport)

        transport.identity_json = _projection(
            "device", "easynet:///r/example/device/dev-a"
        )
        self.assertEqual(
            client.device_ura("example", "dev-a"),
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(
            transport.seen_request,
            {"kind": "device", "realm": "example", "device_id": "dev-a"},
        )

        transport.identity_json = _projection(
            "agent", "easynet:///r/example/agent/alice.caesura"
        )
        self.assertEqual(
            client.agent_ura("example", "alice", "caesura"),
            "easynet:///r/example/agent/alice.caesura",
        )
        self.assertEqual(
            transport.seen_request,
            {
                "kind": "agent",
                "owner_kind": "user",
                "realm": "example",
                "user_id": "alice",
                "agent_id": "caesura",
            },
        )

        transport.identity_json = _projection(
            "agent", "easynet:///r/example/agent/device.dev-a.terminal"
        )
        self.assertEqual(
            client.device_agent_ura("example", "dev-a", "terminal"),
            "easynet:///r/example/agent/device.dev-a.terminal",
        )
        self.assertEqual(
            transport.seen_request,
            {
                "kind": "agent",
                "owner_kind": "device",
                "realm": "example",
                "device_id": "dev-a",
                "agent_id": "terminal",
            },
        )

        transport.identity_json = _projection("hub", "easynet:///r/example/hub")
        self.assertEqual(client.hub_ura("example"), "easynet:///r/example/hub")
        self.assertEqual(transport.seen_request, {"kind": "hub", "realm": "example"})

        transport.identity_json = _projection(
            "resource", "easynet:///r/example/resource/device.dev-a/fs/pkg"
        )
        self.assertEqual(
            client.resource_ura("easynet:///r/example/device/dev-a", "fs/pkg"),
            "easynet:///r/example/resource/device.dev-a/fs/pkg",
        )
        self.assertEqual(
            transport.seen_request,
            {
                "kind": "resource",
                "owner_ura": "easynet:///r/example/device/dev-a",
                "path": "fs/pkg",
            },
        )

    def test_identity_client_exposes_same_delegated_ura_builders(self) -> None:
        transport = MemoryIdentityTransport()
        client = IdentityClient(transport)

        transport.identity_json = _projection(
            "device", "easynet:///r/example/device/dev-a"
        )
        self.assertEqual(
            client.device_ura("example", "dev-a"),
            "easynet:///r/example/device/dev-a",
        )

        transport.identity_json = _projection(
            "agent", "easynet:///r/example/agent/alice.caesura"
        )
        self.assertEqual(
            client.agent_ura("example", "alice", "caesura"),
            "easynet:///r/example/agent/alice.caesura",
        )

        transport.identity_json = _projection(
            "agent", "easynet:///r/example/agent/device.dev-a.terminal"
        )
        self.assertEqual(
            client.device_agent_ura("example", "dev-a", "terminal"),
            "easynet:///r/example/agent/device.dev-a.terminal",
        )

        transport.identity_json = _projection("hub", "easynet:///r/example/hub")
        self.assertEqual(client.hub_ura("example"), "easynet:///r/example/hub")

        transport.identity_json = _projection(
            "resource", "easynet:///r/example/resource/device.dev-a/fs/pkg"
        )
        self.assertEqual(
            client.resource_ura("easynet:///r/example/device/dev-a", "fs/pkg"),
            "easynet:///r/example/resource/device.dev-a/fs/pkg",
        )

        transport.identity_jsons = [
            _projection("device", "easynet:///r/example/device/dev-a"),
            _projection(
                "ability",
                "easynet:///r/example/ability/device.dev-a.observe.health",
            ),
        ]
        self.assertEqual(
            client.device_ability_ura("example", "dev-a", "observe", "health"),
            "easynet:///r/example/ability/device.dev-a.observe.health",
        )
        self.assertEqual(
            transport.seen_requests[-2:],
            [
                {"kind": "device", "realm": "example", "device_id": "dev-a"},
                {
                    "kind": "ability",
                    "owner_ura": "easynet:///r/example/device/dev-a",
                    "ability_name": "observe.health",
                },
            ],
        )

    def test_addressing_helpers_reject_invalid_transport_projection(self) -> None:
        transport = MemoryIdentityTransport()
        transport.identity_json = (
            b'{"kind":"device","valid":true,'
            b'"ura":"easynet:///r/example/device/dev-a",'
            b'"profile":"easynet-strict-v2","components":{},'
            b'"metadata":{"grammar_owner":"axon"}}'
        )
        client = IdentityClient(transport)

        with self.assertRaises(SDKError):
            client.owner_ability_ura(
                "easynet:///r/example/device/dev-a", "observe.health"
            )
        with self.assertRaises(SDKError):
            client.owner_ura_for_ability("easynet:///r/example/device/dev-a")

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

    def test_signing_key_lifecycle_and_signer_handle(self) -> None:
        transport = MemoryIdentityTransport()
        client = IdentityClient(transport)

        record = client.register_signing_key(
            SigningKeyRegistrationRequest(
                owner_ura="easynet:///r/example/agent/alice.sdk",
                key_id="alice-key-1",
                algorithm="ed25519",
                public_key_base64="cHVibGljLWtleQ==",
                usage=("invocation.sign",),
            )
        )
        self.assertEqual(record.key_id, "alice-key-1")
        self.assertEqual(record.usage, ("invocation.sign",))

        page = client.list_signing_keys(
            SigningKeyListRequest(owner_ura="easynet:///r/example/agent/alice.sdk")
        )
        self.assertEqual(page.limit, DEFAULT_SIGNING_KEY_PAGE_SIZE)
        self.assertEqual(page.items[0].key_id, "alice-key-1")
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["limit"], DEFAULT_SIGNING_KEY_PAGE_SIZE)

        revoke = client.revoke_signing_key(
            SigningKeyRevokeRequest(key_id="alice-key-1", reason="rotation")
        )
        self.assertTrue(revoke.revoked)
        self.assertEqual(revoke.state, "revoked")

        signer = client.signer(
            SignerRequest(
                owner_ura="easynet:///r/example/agent/alice.sdk",
                key_id="alice-key-1",
                usage="invocation.sign",
            )
        )
        self.assertEqual(signer.signer_id, "signer-alice-key-1")
        self.assertEqual(signer.algorithm, "ed25519")

    def test_acquire_signer_binds_provider_to_daemon_handle(self) -> None:
        transport = MemoryIdentityTransport()
        client = IdentityClient(transport)

        with self.assertRaises(SDKError) as caught:
            client.acquire_signer(
                SignerRequest(
                    owner_ura="easynet:///r/example/agent/alice.sdk",
                    key_id="alice-key-1",
                    usage="invocation.sign",
                ),
                None,
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(transport.seen_request)

        signer = client.acquire_signer(
            SignerRequest(
                owner_ura="easynet:///r/example/agent/alice.sdk",
                key_id="alice-key-1",
                usage="invocation.sign",
            ),
            StaticSignatureProvider(
                InvocationSignature(
                    algorithm="ed25519",
                    signature_base64="c2lnbmF0dXJl",
                    key_id_hint="signer-alice-key-1",
                )
            ),
        )

        self.assertEqual(signer.handle.signer_id, "signer-alice-key-1")
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["usage"], "invocation.sign")

    def test_signer_handle_rejects_forged_source(self) -> None:
        with self.assertRaises(SDKError) as caught:
            SignerHandle.from_json(
                b"""{
                    "profile":"directory_identity",
                    "signer_id":"signer-forged",
                    "owner_ura":"easynet:///r/example/agent/alice.sdk",
                    "key_id":"alice-key-1",
                    "algorithm":"ed25519",
                    "policy":{"mode":"local_daemon_signing","usage":"invocation.sign"},
                    "metadata":{"source":"product_local_fixture"}
                }"""
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

        with self.assertRaises(SDKError) as caught:
            SignerHandle.from_json(
                b"""{
                    "profile":"directory_identity",
                    "signer_id":"signer-alice-key-1",
                    "owner_ura":"easynet:///r/example/agent/alice.sdk",
                    "key_id":"alice-key-1",
                    "algorithm":"ed25519",
                    "policy":{"mode":"local_daemon_signing","usage":"invocation.sign","signer_id":"other-signer"},
                    "metadata":{"source":"daemon_keyring"}
                }"""
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

        with self.assertRaises(SDKError) as caught:
            SignerHandle.from_json(
                b"""{
                    "profile":"directory_identity",
                    "signer_id":"signer-alice-key-1",
                    "owner_ura":"easynet:///r/example/agent/alice.sdk",
                    "key_id":"alice-key-1",
                    "algorithm":"ed25519",
                    "policy":{"mode":"local_daemon_signing","usage":"invocation.sign"},
                    "metadata":{"source":"daemon_keyring","public_key_base64":"c2hvcnQ="}
                }"""
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_signing_key_lifecycle_rejects_invalid_inputs(self) -> None:
        client = IdentityClient(MemoryIdentityTransport())

        with self.assertRaises(SDKError):
            client.register_signing_key(
                SigningKeyRegistrationRequest(
                    owner_ura="easynet:///r/example/agent/alice.sdk",
                    key_id="alice-key-1",
                    algorithm="ed25519",
                    public_key_base64="cHVibGljLWtleQ==",
                    usage=("invocation.sign",),
                    metadata={"private_key_seed": "must-not-leak"},
                )
            )
        with self.assertRaises(SDKError):
            client.list_signing_keys(
                SigningKeyListRequest(limit=MAX_SIGNING_KEY_PAGE_SIZE + 1)
            )
        with self.assertRaises(SDKError):
            client.revoke_signing_key(SigningKeyRevokeRequest("alice-key-1", ""))

    def test_close_delegates_once_and_fails_closed(self) -> None:
        transport = MemoryIdentityTransport()
        transport.descriptor_json = (
            b'{"kind":"descriptor_ref","valid":true,'
            b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",'
            b'"ability_ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
            b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
            b'"metadata":{}}'
        )
        client = IdentityClient(transport)

        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            client.project_descriptor_ref(
                DescriptorRefRequest(
                    "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
                )
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(transport.seen_request)


SIGNING_KEY_RECORD = (
    b'{"profile":"directory_identity","key_id":"alice-key-1",'
    b'"owner_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"algorithm":"ed25519","public_key_base64":"cHVibGljLWtleQ==",'
    b'"state":"active","usage":["invocation.sign"],'
    b'"created_unix_ms":1783100000123,"metadata":{"source":"daemon_keyring"}}'
)

SIGNING_KEY_PAGE = (
    b'{"profile":"directory_identity","items":['
    b'{"profile":"directory_identity","key_id":"alice-key-1",'
    b'"owner_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"algorithm":"ed25519","public_key_base64":"cHVibGljLWtleQ==",'
    b'"state":"active","usage":["invocation.sign"],'
    b'"created_unix_ms":1783100000123,"metadata":{"source":"daemon_keyring"}}'
    b'],"next_cursor":null,"limit":50,"metadata":{"source":"daemon_keyring"}}'
)

SIGNING_KEY_REVOKE = (
    b'{"profile":"directory_identity","key_id":"alice-key-1",'
    b'"revoked":true,"state":"revoked","metadata":{"reason":"rotation"}}'
)

SIGNER_HANDLE = (
    b'{"profile":"directory_identity","signer_id":"signer-alice-key-1",'
    b'"owner_ura":"easynet:///r/example/agent/alice.sdk","key_id":"alice-key-1",'
    b'"algorithm":"ed25519",'
    b'"policy":{"mode":"local_daemon_signing","usage":"invocation.sign"},'
    b'"metadata":{"source":"daemon_keyring"}}'
)


def _projection(kind: str, ura: str) -> bytes:
    return json.dumps(
        {
            "kind": kind,
            "valid": True,
            "ura": ura,
            "profile": "easynet-strict-v2",
            "components": {},
            "metadata": {"grammar_owner": "axon"},
        },
        separators=(",", ":"),
    ).encode("utf-8")


if __name__ == "__main__":
    unittest.main()
