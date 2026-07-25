import base64
import builtins
import hashlib
import os
import tempfile
import time
import unittest
from unittest import mock

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from easynet_sdk import (
    ErrorCode,
    ManagedSigner,
    ManagedSigningClient,
    ManagedSigningCreateRequest,
    ManagedSigningKeyFilter,
    ManagedSigningPeerRegistration,
    ManagedSigningStatus,
    SDKError,
    SignerHandle,
    SignerPolicy,
    SigningMaterial,
)
from easynet_sdk.providers.runtime.key_service import (
    KEY_SERVICE_PROTOCOL_VERSION,
    MAX_KEY_SERVICE_CANONICAL_BYTES,
    MAX_KEY_SERVICE_FRAME_BYTES,
)
from easynet_sdk.managed_signing import _verify_ed25519_signature

from key_service_fake import KeyServiceResponsePlan, KeyServiceServer


class ManagedSigningTests(unittest.TestCase):
    def test_client_requires_explicit_daemon_endpoint(self) -> None:
        for socket_path in ("", " \t\n "):
            with self.subTest(socket_path=socket_path):
                with self.assertRaises(SDKError) as caught:
                    ManagedSigningClient(socket_path=socket_path)
                self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)
                self.assertEqual(caught.exception.stage, "key_service")

    def test_signature_verification_fails_closed_when_required_crypto_is_unavailable(
        self,
    ) -> None:
        original_import = builtins.__import__

        def reject_cryptography(name: str, *args: object, **kwargs: object) -> object:
            if name.startswith("cryptography"):
                raise ImportError("injected missing cryptography")
            return original_import(name, *args, **kwargs)

        with mock.patch("builtins.__import__", side_effect=reject_cryptography):
            with self.assertRaises(SDKError) as raised:
                _verify_ed25519_signature(bytes(32), b"canonical", bytes(64))

        self.assertEqual(raised.exception.code, ErrorCode.PROTOCOL)

    def test_client_conforms_to_daemon_key_service_protocol(self) -> None:
        private_key_1 = Ed25519PrivateKey.from_private_bytes(bytes(range(32)))
        public_key_1 = private_key_1.public_key().public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
        public_key_2 = bytes(reversed(range(32)))
        peer_public_key = bytes([7] * 32)
        signature = private_key_1.sign(b"canonical")
        fingerprint = hashlib.sha256(peer_public_key).digest()
        key_id_1 = "managed-key-1"
        key_id_2 = "managed-key-2"
        subject_ura = "easynet:///r/acme/agent/signer.main"
        peer_ura = "easynet:///r/peer/agent/verifier.main"
        via_authority_ura = "easynet:///r/acme/authority"
        responses = [
            {
                "result": "inventory_key",
                "entry": _key(key_id_1, public_key_1, "active", 0, None, subject_ura),
            },
            {
                "result": "inventory_keys",
                "entries": [
                    _key(key_id_1, public_key_1, "active", 0, None, subject_ura)
                ],
                "next_cursor": None,
            },
            {
                "result": "inventory_key",
                "entry": _key(key_id_1, public_key_1, "active", 0, None, subject_ura),
            },
            {
                "result": "inventory_key",
                "entry": _key(key_id_1, public_key_1, "active", 0, None, subject_ura),
            },
            {"result": "signature", "signature_b64": _b64(signature)},
            {
                "result": "inventory_key",
                "entry": _key(
                    key_id_2, public_key_2, "active", 1, key_id_1, subject_ura
                ),
            },
            {"result": "inventory_revoked", "revoked_unix_ms": 1700000000100},
            {"result": "ok"},
            {"result": "ok"},
            {"result": "inventory_peer_added", "added": True},
            {
                "result": "inventory_peers",
                "peers": [
                    {
                        "peer_ura": peer_ura,
                        "fingerprint_b64": _b64(fingerprint),
                        "public_key_b64": _b64(peer_public_key),
                        "via_authority": via_authority_ura,
                        "added_unix_ms": 1700000000200,
                        "last_seen_unix_ms": 1700000000300,
                    }
                ],
                "next_cursor": None,
            },
        ]
        with KeyServiceServer(responses) as server:
            client = ManagedSigningClient(socket_path=server.socket_path)
            created = client.create(
                ManagedSigningCreateRequest(
                    purpose="invocation", bound_subject_ura=subject_ura
                )
            )
            listed = client.list(
                ManagedSigningKeyFilter(
                    purpose="invocation", status=ManagedSigningStatus.ACTIVE
                )
            )
            projection = client.public_projection(key_id_1)
            actual_signature = client.sign(key_id_1, b"canonical")
            rotated = client.rotate(key_id_1)
            revoked_at = client.revoke(key_id_1)
            client.set_expiry(key_id_2, 1700000010000)
            client.bind_subject(key_id_2, subject_ura)
            added = client.add_peer(
                ManagedSigningPeerRegistration(
                    peer_ura=peer_ura,
                    public_key=peer_public_key,
                    via_authority_ura=via_authority_ura,
                )
            )
            peers = client.list_peers()

        self.assertEqual(created.key_id, key_id_1)
        self.assertEqual(created.public_key, public_key_1)
        self.assertEqual(created.status, ManagedSigningStatus.ACTIVE)
        self.assertEqual([key.key_id for key in listed], [key_id_1])
        self.assertEqual(
            projection.signer_policy_ref,
            _policy_ref("invocation", subject_ura, key_id_1, public_key_1),
        )
        self.assertEqual(actual_signature, signature)
        self.assertEqual(rotated.rotated_from, key_id_1)
        self.assertEqual(rotated.rotation_epoch, 1)
        self.assertEqual(revoked_at, 1700000000100)
        self.assertTrue(added)
        self.assertEqual(peers[0].peer_ura, peer_ura)
        self.assertEqual(peers[0].public_key, peer_public_key)
        self.assertEqual(peers[0].via_authority_ura, via_authority_ura)

        expected_requests = [
            {
                "method": "inventory.create",
                "purpose": "invocation",
                "bound_subject": subject_ura,
            },
            {
                "method": "inventory.list",
                "purpose": "invocation",
                "status": "active",
                "limit": 16,
            },
            {"method": "inventory.public_key", "key_id": key_id_1},
            {"method": "inventory.public_key", "key_id": key_id_1},
            {
                "method": "inventory.sign",
                "key_id": key_id_1,
                "expected_purpose": "invocation",
                "subject_ura": subject_ura,
                "signer_policy_ref": _policy_ref(
                    "invocation", subject_ura, key_id_1, public_key_1
                ),
                "canonical_bytes_b64": _b64(b"canonical"),
            },
            {"method": "inventory.rotate", "key_id": key_id_1},
            {"method": "inventory.revoke", "key_id": key_id_1},
            {
                "method": "inventory.set_expiry",
                "key_id": key_id_2,
                "expires_unix_ms": 1700000010000,
            },
            {
                "method": "inventory.bind_subject",
                "key_id": key_id_2,
                "subject_ura": subject_ura,
            },
            {
                "method": "inventory.peer_add",
                "peer_ura": peer_ura,
                "public_key_b64": _b64(peer_public_key),
                "via_authority": via_authority_ura,
            },
            {"method": "inventory.peer_list", "limit": 16},
        ]
        self.assertEqual(server.requests, expected_requests)
        for request in server.requests:
            for field in request:
                self.assertFalse(
                    any(
                        token in field.lower()
                        for token in ("seed", "private", "vault", "passphrase")
                    ),
                    field,
                )

    def test_inventory_and_peer_pages_advance_bounded_cursors(self) -> None:
        key_1 = _key("key-1", bytes([1] * 32), "active", 0, None, None)
        key_2 = _key("key-2", bytes([2] * 32), "active", 0, None, None)
        peer_key_1 = bytes([3] * 32)
        peer_key_2 = bytes([4] * 32)
        peer_1 = {
            "peer_ura": "easynet:///r/peer/agent/a",
            "fingerprint_b64": _b64(hashlib.sha256(peer_key_1).digest()),
            "public_key_b64": _b64(peer_key_1),
            "via_authority": None,
            "added_unix_ms": 10,
            "last_seen_unix_ms": 11,
        }
        peer_2 = {
            "peer_ura": "easynet:///r/peer/agent/b",
            "fingerprint_b64": _b64(hashlib.sha256(peer_key_2).digest()),
            "public_key_b64": _b64(peer_key_2),
            "via_authority": None,
            "added_unix_ms": 12,
            "last_seen_unix_ms": 13,
        }
        with KeyServiceServer(
            [
                {
                    "result": "inventory_keys",
                    "entries": [key_1],
                    "next_cursor": "keys:2",
                },
                {
                    "result": "inventory_keys",
                    "entries": [key_2],
                    "next_cursor": None,
                },
                {
                    "result": "inventory_peers",
                    "peers": [peer_1],
                    "next_cursor": "peers:2",
                },
                {
                    "result": "inventory_peers",
                    "peers": [peer_2],
                    "next_cursor": None,
                },
            ]
        ) as server:
            client = ManagedSigningClient(socket_path=server.socket_path)
            keys = client.list()
            peers = client.list_peers()
        self.assertEqual([key.key_id for key in keys], ["key-1", "key-2"])
        self.assertEqual(
            [item.peer_ura for item in peers],
            [peer_1["peer_ura"], peer_2["peer_ura"]],
        )
        self.assertEqual(server.requests[1]["cursor"], "keys:2")
        self.assertEqual(server.requests[3]["cursor"], "peers:2")

    def test_repeated_cursor_is_protocol_error(self) -> None:
        key_1 = _key("cursor-key-1", bytes([1] * 32), "active", 0, None, None)
        key_2 = _key("cursor-key-2", bytes([2] * 32), "active", 0, None, None)
        with KeyServiceServer(
            [
                {
                    "result": "inventory_keys",
                    "entries": [key_1],
                    "next_cursor": "repeat",
                },
                {
                    "result": "inventory_keys",
                    "entries": [key_2],
                    "next_cursor": "repeat",
                },
            ]
        ) as server:
            with self.assertRaises(SDKError) as caught:
                ManagedSigningClient(socket_path=server.socket_path).list()
        self.assertEqual(caught.exception.code, ErrorCode.PROTOCOL)

    def test_empty_filtered_page_can_advance_scan_cursor(self) -> None:
        with KeyServiceServer(
            [
                {
                    "result": "inventory_keys",
                    "entries": [],
                    "next_cursor": "scan-window-1",
                },
                {
                    "result": "inventory_keys",
                    "entries": [],
                    "next_cursor": None,
                },
            ]
        ) as server:
            result = ManagedSigningClient(socket_path=server.socket_path).list()
        self.assertEqual(result, ())
        self.assertEqual(server.requests[1]["cursor"], "scan-window-1")

    def test_key_bound_signer_verifies_signature_and_implements_provider(self) -> None:
        private_key = Ed25519PrivateKey.from_private_bytes(bytes(range(32)))
        public_key = private_key.public_key().public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
        canonical = b"canonical managed signing fixture"
        signature = private_key.sign(canonical)
        key_id = "managed-key-provider"
        subject_ura = "easynet:///r/acme/agent/provider"
        projection = _key(key_id, public_key, "active", 0, None, subject_ura)
        policy_ref = projection["signer_policy_ref"]
        with KeyServiceServer(
            [
                {"result": "inventory_key", "entry": projection},
                {"result": "signature", "signature_b64": _b64(signature)},
                {"result": "signature", "signature_b64": _b64(signature)},
            ]
        ) as server:
            signer = ManagedSigningClient(socket_path=server.socket_path).signer(key_id)
            self.assertIsInstance(signer, ManagedSigner)
            self.assertEqual(signer.key_id, key_id)
            self.assertEqual(signer.signing_public_key(), public_key)
            self.assertEqual(signer.sign_canonical(canonical), signature)
            handle = SignerHandle(
                profile="signing",
                signer_id="signer-managed-key-provider",
                owner_ura=subject_ura,
                key_id=key_id,
                algorithm="ed25519",
                policy={
                    "mode": "provider_managed_signing",
                    "usage": "invocation.sign",
                    "signer_id": "signer-managed-key-provider",
                    "policy_ref": policy_ref,
                    "inventory_owner_ura": subject_ura,
                    "key_state": "active",
                },
                metadata={
                    "source": "provider_key_inventory",
                    "policy_ref": policy_ref,
                    "public_key_base64": _b64(public_key),
                },
            )
            material = SigningMaterial(
                canonical_bytes_base64=_b64(canonical),
                args_digest_hex="00" * 32,
                expires_at_unix_ms=1700000001000,
                algorithm="ed25519",
                signer_policy=SignerPolicy(
                    mode="provider_managed_signing",
                    signer_id=handle.signer_id,
                    policy_ref=str(policy_ref),
                ),
            )
            invocation_signature = signer.sign(material, handle)
        self.assertEqual(
            base64.b64decode(invocation_signature.signature_base64), signature
        )
        self.assertEqual(invocation_signature.key_id_hint, handle.signer_id)
        self.assertEqual(
            invocation_signature.signer_public_key_base64, _b64(public_key)
        )

    def test_key_bound_signer_rejects_invalid_daemon_signature(self) -> None:
        private_key = Ed25519PrivateKey.from_private_bytes(bytes([8] * 32))
        public_key = private_key.public_key().public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
        with KeyServiceServer(
            [
                {
                    "result": "inventory_key",
                    "entry": _key(
                        "managed-key-invalid-signature",
                        public_key,
                    "active",
                    0,
                    None,
                    "easynet:///r/acme/agent/signer.main",
                ),
                },
                {"result": "signature", "signature_b64": _b64(bytes(64))},
            ]
        ) as server:
            signer = ManagedSigningClient(socket_path=server.socket_path).signer(
                "managed-key-invalid-signature"
            )
            with self.assertRaises(SDKError) as caught:
                signer.sign_canonical(b"canonical")
        self.assertEqual(caught.exception.code, ErrorCode.PROTOCOL)

    def test_response_allowlists_reject_private_and_unknown_fields(self) -> None:
        public_key = bytes(range(32))
        private_projection = _key(
            "managed-key-private", public_key, "active", 0, None, None
        )
        private_projection["private_key_seed"] = "forbidden"
        cases = [
            {
                "result": "inventory_key",
                "entry": private_projection,
            },
            {
                "result": "inventory_key",
                "entry": _key(
                    "managed-key-unknown", public_key, "active", 0, None, None
                ),
                "unexpected": True,
            },
        ]
        for response in cases:
            with self.subTest(response=response):
                with KeyServiceServer([response]) as server:
                    with self.assertRaises(SDKError) as caught:
                        ManagedSigningClient(
                            socket_path=server.socket_path
                        ).public_projection(str(response["entry"]["key_id"]))
                self.assertEqual(caught.exception.code, ErrorCode.PROTOCOL)

    def test_projection_width_lifecycle_and_peer_hash_invariants(self) -> None:
        public_key = bytes(range(32))
        overflow = _key("overflow", public_key, "active", 0, None, None)
        overflow["rotation_epoch"] = 1 << 64
        zero_expiry = _key("zero-expiry", public_key, "active", 0, None, None)
        zero_expiry["expires_unix_ms"] = 0
        revoked_without_timestamp = _key(
            "revoked", public_key, "revoked", 0, None, None
        )
        revoked_without_timestamp["revoked_unix_ms"] = None
        peer_key = bytes([4] * 32)
        invalid_peer = {
            "peer_ura": "easynet:///r/peer/agent/hash",
            "fingerprint_b64": _b64(bytes(32)),
            "public_key_b64": _b64(peer_key),
            "via_authority": None,
            "added_unix_ms": 10,
            "last_seen_unix_ms": 11,
        }
        calls = [
            (
                {"result": "inventory_key", "entry": overflow},
                lambda client: client.public_projection("overflow"),
            ),
            (
                {"result": "inventory_key", "entry": zero_expiry},
                lambda client: client.public_projection("zero-expiry"),
            ),
            (
                {"result": "inventory_key", "entry": revoked_without_timestamp},
                lambda client: client.public_projection("revoked"),
            ),
            (
                {
                    "result": "inventory_peers",
                    "peers": [invalid_peer],
                    "next_cursor": None,
                },
                lambda client: client.list_peers(),
            ),
        ]
        for response, call in calls:
            with self.subTest(response=response):
                with KeyServiceServer([response]) as server:
                    with self.assertRaises(SDKError) as caught:
                        call(ManagedSigningClient(socket_path=server.socket_path))
                self.assertEqual(caught.exception.code, ErrorCode.PROTOCOL)

        for invalid_expiry in (0, -1, 1 << 63):
            with self.subTest(invalid_expiry=invalid_expiry):
                with self.assertRaises(SDKError) as caught:
                    ManagedSigningClient(socket_path="/unused/keyring.sock").set_expiry(
                        "key", invalid_expiry
                    )
                self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

    def test_projection_policy_is_purpose_bound(self) -> None:
        entry = _key(
            "purpose-bound",
            bytes(range(32)),
            "active",
            0,
            None,
            "easynet:///r/acme/agent/signer",
        )
        entry["purpose"] = "different-purpose"
        with KeyServiceServer(
            [{"result": "inventory_key", "entry": entry}]
        ) as server:
            with self.assertRaises(SDKError) as caught:
                ManagedSigningClient(
                    socket_path=server.socket_path
                ).public_projection("purpose-bound")
        self.assertEqual(caught.exception.code, ErrorCode.PROTOCOL)

    def test_lifecycle_and_io_rejections_are_typed(self) -> None:
        cases = [
            ("lifecycle", ErrorCode.POLICY_DENIED),
            ("io", ErrorCode.EXECUTION_FAILED),
            ("durability_uncertain", ErrorCode.EXECUTION_FAILED),
            ("fail_stopped", ErrorCode.EXECUTION_FAILED),
        ]
        for kind, code in cases:
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
                    client = ManagedSigningClient(socket_path=server.socket_path)
                    with self.assertRaises(SDKError) as caught:
                        client.sign("retired-key", b"canonical")
                self.assertEqual(caught.exception.code, code)
                self.assertFalse(caught.exception.retryable)
                self.assertEqual(caught.exception.details.get("kind"), kind)

    def test_peer_key_replacement_policy_rejection_is_propagated(self) -> None:
        with KeyServiceServer(
            [
                {
                    "result": "error",
                    "kind": "policy",
                    "message": "explicit retrust is required",
                }
            ]
        ) as server:
            client = ManagedSigningClient(socket_path=server.socket_path)
            with self.assertRaises(SDKError) as caught:
                client.add_peer(
                    ManagedSigningPeerRegistration(
                        peer_ura="easynet:///r/peer/agent/verifier.main",
                        public_key=bytes([1] * 32),
                    )
                )
        self.assertEqual(caught.exception.code, ErrorCode.POLICY_DENIED)
        self.assertEqual(caught.exception.details.get("kind"), "policy")

    def test_connect_is_offline_but_post_connect_eof_is_transport(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            client = ManagedSigningClient(
                socket_path=os.path.join(directory, "missing.sock"),
                timeout_seconds=0.2,
            )
            with self.assertRaises(SDKError) as offline:
                client.sign("key", b"canonical")
        self.assertEqual(offline.exception.code, ErrorCode.RUNTIME_OFFLINE)

        with KeyServiceServer([KeyServiceResponsePlan(None)]) as server:
            with self.assertRaises(SDKError) as transport:
                ManagedSigningClient(
                    socket_path=server.socket_path, timeout_seconds=0.2
                ).sign("key", b"canonical")
        self.assertEqual(transport.exception.code, ErrorCode.TRANSPORT)
        self.assertTrue(transport.exception.retryable)

    def test_slow_drip_response_obeys_one_absolute_deadline(self) -> None:
        response = {"result": "signature", "signature_b64": _b64(bytes(64))}
        started = time.monotonic()
        with KeyServiceServer(
            [
                KeyServiceResponsePlan(
                    response,
                    chunk_size=1,
                    chunk_delay_seconds=0.015,
                )
            ]
        ) as server:
            with self.assertRaises(SDKError) as caught:
                ManagedSigningClient(
                    socket_path=server.socket_path, timeout_seconds=0.05
                ).sign("key", b"canonical")
        elapsed = time.monotonic() - started
        self.assertEqual(caught.exception.code, ErrorCode.TRANSPORT)
        self.assertLess(elapsed, 0.3)

    def test_frame_contract_carries_full_canonical_signing_limit(self) -> None:
        self.assertEqual(KEY_SERVICE_PROTOCOL_VERSION, 2)
        encoded_canonical_bytes = 4 * ((MAX_KEY_SERVICE_CANONICAL_BYTES + 2) // 3)
        self.assertGreaterEqual(
            MAX_KEY_SERVICE_FRAME_BYTES, encoded_canonical_bytes + 1024
        )

    def test_canonical_projection_fixture_matches_go_sdk(self) -> None:
        private_key = Ed25519PrivateKey.from_private_bytes(bytes([1] * 32))
        public_key = private_key.public_key().public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
        subject_ura = "easynet:///r/acme/agent/signer"
        key_id = "managed-key-1"
        entry = _key(key_id, public_key, "active", 0, None, subject_ura)
        self.assertEqual(
            entry["signer_policy_ref"],
            "managed-signing:v2:sha256:e7e82ca6208b6a4ebf2369739a2c260a",
        )
        fingerprint = hashlib.sha256(public_key).digest()
        self.assertEqual(
            _b64(fingerprint),
            "NHUPmL1Z/PyUbaRaqr6TO+FUpLUJThxKv0KGZQXzyX4=",
        )
        peer = {
            "peer_ura": "easynet:///r/peer/agent/fixture",
            "fingerprint_b64": _b64(fingerprint),
            "public_key_b64": _b64(public_key),
            "via_authority": None,
            "added_unix_ms": 1,
            "last_seen_unix_ms": 1,
        }
        with KeyServiceServer(
            [
                {"result": "inventory_key", "entry": entry},
                {
                    "result": "inventory_peers",
                    "peers": [peer],
                    "next_cursor": None,
                },
            ]
        ) as server:
            client = ManagedSigningClient(socket_path=server.socket_path)
            self.assertEqual(client.public_projection(key_id).public_key, public_key)
            self.assertEqual(client.list_peers()[0].fingerprint, fingerprint)

    def test_wrong_response_variant_is_protocol_error(self) -> None:
        with KeyServiceServer(
            [{"result": "signature", "signature_b64": _b64(bytes(64))}]
        ) as server:
            client = ManagedSigningClient(socket_path=server.socket_path)
            with self.assertRaises(SDKError) as caught:
                client.public_projection("managed-key-1")
        self.assertEqual(caught.exception.code, ErrorCode.PROTOCOL)


def _key(
    key_id: str,
    public_key: bytes,
    status: str,
    rotation_epoch: int,
    rotated_from: str | None,
    subject_ura: str | None,
) -> dict[str, object]:
    revoked_unix_ms = 1700000000100 if status == "revoked" else None
    return {
        "key_id": key_id,
        "purpose": "invocation",
        "public_key_b64": _b64(public_key),
        "status": status,
        "rotation_epoch": rotation_epoch,
        "bound_subject": subject_ura,
        "signer_policy_ref": (
            _policy_ref("invocation", subject_ura, key_id, public_key)
            if subject_ura is not None
            else None
        ),
        "rotated_from": rotated_from,
        "created_unix_ms": 1700000000000,
        "expires_unix_ms": None,
        "revoked_unix_ms": revoked_unix_ms,
    }


def _policy_ref(
    purpose: str, subject_ura: str, key_id: str, public_key: bytes
) -> str:
    digest = hashlib.sha256()
    for component in (
        "canonical-runtime.managed-signing.policy",
        "v2",
        purpose,
        subject_ura,
        key_id,
        _b64(public_key),
    ):
        digest.update(component.encode())
        digest.update(b"\0")
    return f"managed-signing:v2:sha256:{digest.hexdigest()[:32]}"


def _b64(value: bytes) -> str:
    return base64.b64encode(value).decode("ascii")


if __name__ == "__main__":
    unittest.main()
