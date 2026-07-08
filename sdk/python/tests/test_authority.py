import base64
import json
import unittest

from easynet_sdk import (
    AuthorityClient,
    DELEGATION_METADATA_KEY,
    SESSION_AUTHORITY_METADATA_KEY,
    DelegationRequest,
    DelegationProof,
    ErrorCode,
    InvocationBuilder,
    SDKError,
    SessionAuthorityRequest,
    SessionAuthority,
    AuthoritySignature,
    is_code,
)
from easynet_sdk._cabi import CABIAuthorityTransport


class AuthorityTests(unittest.TestCase):
    def test_delegation_metadata_projects_typed_authority(self) -> None:
        value = _authority_metadata(
            {
                "issuer_ura": "easynet:///r/example/user/alice",
                "subject_ura": "easynet:///r/example/user/alice",
                "caller_ura": "easynet:///r/example/agent/backend",
                "audience": "easynet:///r/example/device/dev-a",
                "scopes": ["device.observe.*"],
                "issued_at_ms": 1000,
                "expires_at_ms": 2000,
            },
            b"delegation-signature",
        )

        proof = DelegationProof.from_metadata(value)

        self.assertEqual(proof.issuer_ura, "easynet:///r/example/user/alice")
        self.assertEqual(proof.caller_ura, "easynet:///r/example/agent/backend")
        self.assertEqual(proof.signature, b"delegation-signature")
        metadata = proof.metadata()
        self.assertEqual(metadata.key, DELEGATION_METADATA_KEY)
        self.assertEqual(metadata.value, value)

    def test_session_authority_metadata_projects_typed_authority(self) -> None:
        value = _authority_metadata(_session_authority_payload(), b"session-signature")

        authority = SessionAuthority.from_metadata(value)

        self.assertEqual(authority.issuer_ura, "easynet:///r/example/agent/backend")
        self.assertEqual(authority.session_id, "session-1")
        self.assertEqual(authority.subject_ura, "easynet:///r/example/session/session-1")
        self.assertEqual(authority.signature, b"session-signature")
        metadata = authority.metadata()
        self.assertEqual(metadata.key, SESSION_AUTHORITY_METADATA_KEY)
        self.assertEqual(metadata.value, value)

    def test_invocation_builder_attaches_one_authority_metadata(self) -> None:
        proof = DelegationProof.from_metadata(
            _authority_metadata(
                {
                    "issuer_ura": "easynet:///r/example/user/alice",
                    "subject_ura": "easynet:///r/example/user/alice",
                    "caller_ura": "easynet:///r/example/agent/backend",
                    "audience": "*",
                    "scopes": ["*"],
                    "issued_at_ms": 1000,
                    "expires_at_ms": 2000,
                },
                b"signature",
            )
        )

        draft = (
            InvocationBuilder()
            .with_caller_ura("easynet:///r/example/agent/backend")
            .with_callee_ura("easynet:///r/example/device/dev-a")
            .with_descriptor_ref(
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            )
            .with_subject_ura("easynet:///r/example/user/alice")
            .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
            .with_causal_context({"form": "none"})
            .with_json_args({})
            .with_content_type("application/json")
            .with_metadata({"trace": "t-1"})
            .with_authority_metadata(proof.metadata())
            .build()
        )

        self.assertEqual(draft.metadata["trace"], "t-1")
        self.assertEqual(draft.metadata[DELEGATION_METADATA_KEY], proof.metadata().value)

    def test_invocation_builder_rejects_ambiguous_authority_metadata(self) -> None:
        with self.assertRaises(SDKError) as caught:
            (
                InvocationBuilder()
                .with_caller_ura("easynet:///r/example/agent/backend")
                .with_callee_ura("easynet:///r/example/device/dev-a")
                .with_descriptor_ref(
                    "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
                )
                .with_subject_ura("easynet:///r/example/user/alice")
                .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
                .with_causal_context({"form": "none"})
                .with_json_args({})
                .with_content_type("application/json")
                .with_metadata(
                    {
                        DELEGATION_METADATA_KEY: "delegation",
                        SESSION_AUTHORITY_METADATA_KEY: "session",
                    }
                )
                .build()
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_authority_client_mints_delegation_through_transport(self) -> None:
        value = _authority_metadata(
            {
                "issuer_ura": "easynet:///r/example/user/alice",
                "subject_ura": "easynet:///r/example/user/alice",
                "caller_ura": "easynet:///r/example/agent/backend",
                "audience": "easynet:///r/example/device/dev-a",
                "scopes": ["device.observe.*"],
                "issued_at_ms": 1000,
                "expires_at_ms": 2000,
            },
            b"delegation-signature",
        )
        transport = _MemoryAuthorityTransport(
            delegation_json=json.dumps({"metadata_value": value}).encode("utf-8")
        )
        client = AuthorityClient(transport)

        proof = client.mint_delegation_proof(
            DelegationRequest(
                issuer_ura="easynet:///r/example/user/alice",
                subject_ura="easynet:///r/example/user/alice",
                caller_ura="easynet:///r/example/agent/backend",
                audience="easynet:///r/example/device/dev-a",
                scopes=("device.observe.*",),
                issued_at_ms=1000,
                expires_at_ms=2000,
            )
        )

        self.assertEqual(proof.caller_ura, "easynet:///r/example/agent/backend")
        self.assertEqual(proof.metadata().value, value)
        self.assertEqual(
            transport.seen_delegation["caller_ura"],
            "easynet:///r/example/agent/backend",
        )

    def test_authority_client_mints_session_through_transport(self) -> None:
        value = _authority_metadata(_session_authority_payload(), b"session-signature")
        transport = _MemoryAuthorityTransport(
            session_json=json.dumps(
                {"metadata": {SESSION_AUTHORITY_METADATA_KEY: value}}
            ).encode("utf-8")
        )
        client = AuthorityClient(transport)

        authority = client.mint_session_authority(
            SessionAuthorityRequest(
                issuer_ura="easynet:///r/example/agent/backend",
                session_id="session-1",
                session_owner_user_id="alice",
                creator_principal_id="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/session/session-1",
                audience="easynet:///r/example/device/dev-a",
                scopes=("device.observe.*",),
                allowed_actions=("read",),
                allowed_followup_abilities=("device.observe.health",),
                issued_at_ms=1000,
                expires_at_ms=2000,
            )
        )

        self.assertEqual(authority.audience, "easynet:///r/example/device/dev-a")
        self.assertEqual(authority.metadata().value, value)
        self.assertEqual(transport.seen_session["audience"], "easynet:///r/example/device/dev-a")

    def test_authority_client_rejects_invalid_mint_before_transport(self) -> None:
        transport = _MemoryAuthorityTransport()
        client = AuthorityClient(transport)

        with self.assertRaises(SDKError) as caught:
            client.mint_delegation_proof(
                DelegationRequest(
                    issuer_ura="easynet:///r/example/user/alice",
                    subject_ura="easynet:///r/example/user/alice",
                    caller_ura="easynet:///r/example/agent/backend",
                    audience="easynet:///r/example/device/dev-a",
                    scopes=(),
                    issued_at_ms=1000,
                    expires_at_ms=2000,
                )
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(transport.delegation_calls, 0)

        with self.assertRaises(SDKError) as caught:
            client.mint_session_authority(
                SessionAuthorityRequest(
                    issuer_ura="easynet:///r/example/agent/backend",
                    session_id="session-1",
                    session_owner_user_id="alice",
                    creator_principal_id="easynet:///r/example/agent/backend",
                    callee_ura="easynet:///r/example/device/dev-a",
                    subject_ura="easynet:///r/example/session/session-1",
                    audience="easynet:///r/example/device/dev-a",
                    scopes=("device.observe.*",),
                    allowed_actions=("read",),
                    allowed_followup_abilities=("device.observe.health",),
                    issued_at_ms=2000,
                    expires_at_ms=1000,
                )
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(transport.session_calls, 0)

    def test_cabi_authority_transport_mints_via_core_and_signer(self) -> None:
        delegation_value = _authority_metadata(
            {
                "issuer_ura": "easynet:///r/example/user/alice",
                "subject_ura": "easynet:///r/example/user/alice",
                "caller_ura": "easynet:///r/example/agent/backend",
                "audience": "easynet:///r/example/device/dev-a",
                "scopes": ["device.observe.*"],
                "issued_at_ms": 1000,
                "expires_at_ms": 2000,
            },
            b"cabi-signature",
        )
        session_value = _authority_metadata(_session_authority_payload(), b"cabi-signature")
        signer = _RecordingAuthoritySigner(
            AuthoritySignature(
                signature_base64=base64.b64encode(b"cabi-signature").decode("ascii")
            )
        )
        transport = CABIAuthorityTransport(
            lib=_FakeCABIAuthorityLibrary(delegation_value, session_value),
            signer=signer,
        )
        client = AuthorityClient(transport)

        proof = client.mint_delegation_proof(
            DelegationRequest(
                issuer_ura="easynet:///r/example/user/alice",
                subject_ura="easynet:///r/example/user/alice",
                caller_ura="easynet:///r/example/agent/backend",
                audience="easynet:///r/example/device/dev-a",
                scopes=("device.observe.*",),
                issued_at_ms=1000,
                expires_at_ms=2000,
            )
        )
        session = client.mint_session_authority(
            SessionAuthorityRequest(
                issuer_ura="easynet:///r/example/agent/backend",
                session_id="session-1",
                session_owner_user_id="alice",
                creator_principal_id="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/session/session-1",
                audience="easynet:///r/example/device/dev-a",
                scopes=("device.observe.*",),
                allowed_actions=("read",),
                allowed_followup_abilities=("device.observe.health",),
                issued_at_ms=1000,
                expires_at_ms=2000,
            )
        )

        self.assertEqual(proof.metadata().value, delegation_value)
        self.assertEqual(session.metadata().value, session_value)
        self.assertEqual(signer.seen[0].kind, "delegation")
        self.assertEqual(signer.seen[1].kind, "session_authority")

    def test_cabi_authority_transport_rejects_non_latest_signature(self) -> None:
        class BadSigner:
            def sign_authority(self, material):
                return object()

        transport = CABIAuthorityTransport(
            lib=_FakeCABIAuthorityLibrary("delegation", "session"),
            signer=BadSigner(),
        )
        with self.assertRaises(SDKError) as caught:
            transport.mint_delegation_proof(
                b"""{
                "issuer_ura":"easynet:///r/example/user/alice",
                "subject_ura":"easynet:///r/example/user/alice",
                "caller_ura":"easynet:///r/example/agent/backend",
                "audience":"easynet:///r/example/device/dev-a",
                "scopes":["device.observe.*"],
                "issued_at_ms":1000,
                "expires_at_ms":2000
                }"""
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.ROUTE_UNAVAILABLE))


def _authority_metadata(payload: dict[str, object], signature: bytes) -> str:
    wire = json.dumps(
        {
            "payload": payload,
            "signature": base64.b64encode(signature).decode("ascii"),
        },
        separators=(",", ":"),
    ).encode("utf-8")
    return base64.b64encode(wire).decode("ascii")


def _session_authority_payload() -> dict[str, object]:
    return {
        "issuer_ura": "easynet:///r/example/agent/backend",
        "session_id": "session-1",
        "session_owner_user_id": "alice",
        "creator_principal_id": "easynet:///r/example/agent/backend",
        "callee_ura": "easynet:///r/example/device/dev-a",
        "subject_ura": "easynet:///r/example/session/session-1",
        "audience": "easynet:///r/example/device/dev-a",
        "scopes": ["device.observe.*"],
        "allowed_actions": ["read"],
        "allowed_followup_abilities": ["device.observe.health"],
        "issued_at_ms": 1000,
        "expires_at_ms": 2000,
    }


class _MemoryAuthorityTransport:
    def __init__(
        self,
        *,
        delegation_json: bytes = b"",
        session_json: bytes = b"",
    ) -> None:
        self.delegation_json = delegation_json
        self.session_json = session_json
        self.delegation_calls = 0
        self.session_calls = 0
        self.seen_delegation: dict[str, object] = {}
        self.seen_session: dict[str, object] = {}

    def mint_delegation_proof(self, request_json: bytes) -> bytes:
        self.delegation_calls += 1
        self.seen_delegation = json.loads(request_json.decode("utf-8"))
        return self.delegation_json

    def mint_session_authority(self, request_json: bytes) -> bytes:
        self.session_calls += 1
        self.seen_session = json.loads(request_json.decode("utf-8"))
        return self.session_json


class _RecordingAuthoritySigner:
    def __init__(self, signature: AuthoritySignature) -> None:
        self.signature = signature
        self.seen = []

    def sign_authority(self, material):
        self.seen.append(material)
        return self.signature


class _FakeCABIAuthorityLibrary:
    def __init__(self, delegation_value: str, session_value: str) -> None:
        self.delegation_value = delegation_value
        self.session_value = session_value
        self.seen_delegation_signature: dict[str, object] = {}
        self.seen_session_signature: dict[str, object] = {}

    def authority_prepare_delegation(self, request_json: bytes) -> bytes:
        json.loads(request_json.decode("utf-8"))
        return json.dumps(
            {
                "profile": "authority",
                "kind": "delegation",
                "algorithm": "ed25519",
                "metadata_key": DELEGATION_METADATA_KEY,
                "canonical_bytes_base64": base64.b64encode(b"canonical").decode("ascii"),
                "canonical_hash_hex": "a" * 64,
                "signed_fields": ["issuer_ura"],
                "payload": {"issuer_ura": "easynet:///r/example/user/alice"},
            }
        ).encode("utf-8")

    def authority_materialize_delegation(
        self, request_json: bytes, signature_json: bytes
    ) -> bytes:
        json.loads(request_json.decode("utf-8"))
        self.seen_delegation_signature = json.loads(signature_json.decode("utf-8"))
        return json.dumps(
            {
                "metadata_value": self.delegation_value,
                "metadata": {DELEGATION_METADATA_KEY: self.delegation_value},
            }
        ).encode("utf-8")

    def authority_prepare_session(self, request_json: bytes) -> bytes:
        request = json.loads(request_json.decode("utf-8"))
        return json.dumps(
            {
                "profile": "authority",
                "kind": "session_authority",
                "algorithm": "ed25519",
                "metadata_key": SESSION_AUTHORITY_METADATA_KEY,
                "canonical_bytes_base64": base64.b64encode(b"canonical").decode("ascii"),
                "canonical_hash_hex": "b" * 64,
                "signed_fields": [
                    "issuer_ura",
                    "session_id",
                    "session_owner_user_id",
                    "creator_principal_id",
                    "callee_ura",
                    "subject_ura",
                    "audience",
                    "scopes",
                    "allowed_actions",
                    "allowed_followup_abilities",
                    "issued_at_ms",
                    "expires_at_ms",
                ],
                "payload": {
                    "issuer_ura": request["issuer_ura"],
                    "session_id": request["session_id"],
                    "session_owner_user_id": request["session_owner_user_id"],
                    "creator_principal_id": request["creator_principal_id"],
                    "callee_ura": request["callee_ura"],
                    "subject_ura": request["subject_ura"],
                    "audience": request["audience"],
                    "scopes": request["scopes"],
                    "allowed_actions": request["allowed_actions"],
                    "allowed_followup_abilities": request["allowed_followup_abilities"],
                    "issued_at_ms": request["issued_at_ms"],
                    "expires_at_ms": request["expires_at_ms"],
                },
            }
        ).encode("utf-8")

    def authority_materialize_session(
        self, request_json: bytes, signature_json: bytes
    ) -> bytes:
        json.loads(request_json.decode("utf-8"))
        self.seen_session_signature = json.loads(signature_json.decode("utf-8"))
        return json.dumps(
            {
                "metadata_value": self.session_value,
                "metadata": {SESSION_AUTHORITY_METADATA_KEY: self.session_value},
            }
        ).encode("utf-8")


if __name__ == "__main__":
    unittest.main()
