import base64
import hashlib
import unittest

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from easynet_sdk import ErrorCode, SDKError
from easynet_sdk.runtime_identity import (
    default_runtime_keyring_socket_path,
    ensure_runtime_signing_identity,
    load_runtime_signing_identity,
)
from tests.key_service_fake import KeyServiceServer


class RuntimeIdentityTests(unittest.TestCase):
    def test_runtime_identity_requires_explicit_daemon_endpoint(self) -> None:
        with self.assertRaises(SDKError) as caught:
            default_runtime_keyring_socket_path()
        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)
        self.assertEqual(caught.exception.stage, "runtime_identity")

        for socket_path in ("", " \t\n "):
            with self.subTest(socket_path=socket_path):
                with self.assertRaises(SDKError) as caught:
                    load_runtime_signing_identity(
                        "easynet:///r/acme/hub", socket_path=socket_path
                    )
                self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)
                self.assertEqual(caught.exception.stage, "key_service")

                with self.assertRaises(SDKError) as caught:
                    ensure_runtime_signing_identity(
                        "easynet:///r/acme/hub", socket_path=socket_path
                    )
                self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)
                self.assertEqual(caught.exception.stage, "key_service")

    def test_load_and_sign_use_daemon_keyring_protocol(self) -> None:
        private_key = Ed25519PrivateKey.from_private_bytes(bytes(range(32)))
        public_key = private_key.public_key().public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
        signature = private_key.sign(b"canonical")
        requests: list[dict[str, object]] = []
        with KeyServiceServer(
            [
                {
                    "result": "public_key",
                    "public_key_b64": base64.b64encode(public_key).decode(),
                },
                {
                    "result": "signature",
                    "signature_b64": base64.b64encode(signature).decode(),
                },
            ]
        ) as server:
            identity = load_runtime_signing_identity(
                "easynet:///r/acme/hub", socket_path=server.socket_path
            )
            self.assertEqual(identity.public_key, public_key)
            self.assertEqual(identity.sign_canonical(b"canonical"), signature)
            requests.extend(server.requests)
        self.assertEqual(
            requests[0],
            {"method": "derive_pubkey", "self_ura": "easynet:///r/acme/hub"},
        )
        self.assertEqual(requests[1]["method"], "sign")
        public_key_b64 = base64.b64encode(public_key).decode("ascii")
        expected_policy = hashlib.sha256(
            b"easynet:///r/acme/hub\0easynet:///r/acme/hub\0"
            + public_key_b64.encode("ascii")
        ).hexdigest()[:32]
        self.assertEqual(requests[1]["public_key_b64"], public_key_b64)
        self.assertEqual(
            requests[1]["signer_policy_ref"],
            f"daemon-key-inventory:sha256:{expected_policy}",
        )
        self.assertNotIn("private_key_seed", requests[1])
        self.assertNotIn("vault_path", requests[1])

    def test_ensure_delegates_single_owner_key_generation_to_daemon(self) -> None:
        public_key = bytes(range(32))
        requests: list[dict[str, object]] = []
        with KeyServiceServer(
            [
                {
                    "result": "public_key",
                    "public_key_b64": base64.b64encode(public_key).decode(),
                }
            ]
        ) as server:
            identity = ensure_runtime_signing_identity(
                "easynet:///r/acme/hub",
                socket_path=server.socket_path,
            )
            requests.extend(server.requests)
        self.assertEqual(identity.public_key, public_key)
        self.assertEqual(
            requests[0],
            {"method": "ensure", "primary_self": "easynet:///r/acme/hub"},
        )
        self.assertNotIn("seed_hex", requests[0])

    def test_runtime_identity_rejects_signature_from_another_key(self) -> None:
        owner_key = Ed25519PrivateKey.from_private_bytes(bytes(range(32)))
        owner_public_key = owner_key.public_key().public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
        wrong_key = Ed25519PrivateKey.from_private_bytes(bytes([9] * 32))
        with KeyServiceServer(
            [
                {
                    "result": "public_key",
                    "public_key_b64": base64.b64encode(owner_public_key).decode(),
                },
                {
                    "result": "signature",
                    "signature_b64": base64.b64encode(
                        wrong_key.sign(b"canonical")
                    ).decode(),
                },
            ]
        ) as server:
            identity = load_runtime_signing_identity(
                "easynet:///r/acme/hub", socket_path=server.socket_path
            )
            with self.assertRaises(SDKError) as caught:
                identity.sign_canonical(b"canonical")
        self.assertEqual(caught.exception.code, ErrorCode.PROTOCOL)

    def test_malformed_response_preserves_runtime_identity_error_contract(self) -> None:
        with KeyServiceServer(
            [{"result": "public_key", "public_key_b64": "not-base64"}]
        ) as server:
            with self.assertRaises(SDKError) as caught:
                load_runtime_signing_identity(
                    "easynet:///r/acme/hub", socket_path=server.socket_path
                )
        self.assertEqual(caught.exception.code, ErrorCode.PROTOCOL)
        self.assertEqual(caught.exception.stage, "runtime_identity")

    def test_rejection_compatibility_maps_not_found_and_permission(self) -> None:
        cases = [
            ("not_found", ErrorCode.NOT_FOUND),
            ("policy", ErrorCode.POLICY_DENIED),
            ("io", ErrorCode.EXECUTION_FAILED),
        ]
        for kind, expected_code in cases:
            with self.subTest(kind=kind):
                with KeyServiceServer(
                    [
                        {
                            "result": "error",
                            "kind": kind,
                            "message": "rejected",
                        }
                    ]
                ) as server:
                    with self.assertRaises(SDKError) as caught:
                        load_runtime_signing_identity(
                            "easynet:///r/acme/hub", socket_path=server.socket_path
                        )
                self.assertEqual(caught.exception.code, expected_code)
                self.assertEqual(caught.exception.stage, "runtime_identity")

    def test_unknown_response_field_is_invalid_runtime_identity_input(self) -> None:
        with KeyServiceServer(
            [
                {
                    "result": "public_key",
                    "public_key_b64": base64.b64encode(bytes(32)).decode(),
                    "unexpected": True,
                }
            ]
        ) as server:
            with self.assertRaises(SDKError) as caught:
                load_runtime_signing_identity(
                    "easynet:///r/acme/hub", socket_path=server.socket_path
                )
        self.assertEqual(caught.exception.code, ErrorCode.PROTOCOL)
        self.assertEqual(caught.exception.stage, "runtime_identity")
