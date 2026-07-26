import base64
import json
import unittest

import easynet_sdk.authority as authority_module
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
    is_code,
    new_canonical_authority_client,
)


class AuthorityTests(unittest.TestCase):
    def test_canonical_authority_client_mints_with_opaque_signer(self) -> None:
        signer = _CanonicalSigner()
        client = new_canonical_authority_client(signer)

        authority = client.mint_session_authority(
            SessionAuthorityRequest(
                issuer_ura="easynet:///r/example/agent/backend",
                session_id="session-1",
                session_owner_user_id="alice",
                creator_principal_id="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/resource/user.alice/session/session-1",
                audience="easynet:///r/example/device/dev-a",
                scopes=("device.observe.*",),
                allowed_actions=("read",),
                allowed_followup_abilities=("device.observe.health",),
                issued_at_ms=1000,
                expires_at_ms=2000,
            )
        )

        self.assertEqual(authority.audience, "easynet:///r/example/device/dev-a")
        self.assertEqual(len(authority.signature), 64)
        self.assertEqual(len(signer.payloads), 1)

        client.close()
        with self.assertRaises(SDKError):
            client.mint_session_authority(
                SessionAuthorityRequest(
                    issuer_ura="easynet:///r/example/agent/backend",
                    session_id="session-2",
                    session_owner_user_id="alice",
                    creator_principal_id="easynet:///r/example/agent/backend",
                    callee_ura="easynet:///r/example/device/dev-a",
                    subject_ura="easynet:///r/example/resource/user.alice/session/session-2",
                    audience="easynet:///r/example/device/dev-a",
                    scopes=("device.observe.*",),
                    allowed_actions=("read",),
                    allowed_followup_abilities=("device.observe.health",),
                    issued_at_ms=1000,
                    expires_at_ms=2000,
                )
            )

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
        self.assertEqual(
            authority.subject_ura,
            "easynet:///r/example/resource/user.alice/session/session-1",
        )
        self.assertEqual(authority.session_owner_ura, "easynet:///r/example/user/alice")
        self.assertEqual(authority.creator_principal_ura, "")
        self.assertEqual(authority.signature, b"session-signature")
        metadata = authority.metadata()
        self.assertEqual(metadata.key, SESSION_AUTHORITY_METADATA_KEY)
        self.assertEqual(metadata.value, value)

    def test_session_authority_rejects_all_zero_owner(self) -> None:
        payload = _session_authority_payload()
        payload["session_owner_user_id"] = "00000000-0000-0000-0000-000000000000"
        value = _authority_metadata(payload, b"session-signature")

        with self.assertRaisesRegex(SDKError, "session_owner_user_id must not be all-zero"):
            SessionAuthority.from_metadata(value)

    def test_session_authority_binds_canonical_subject(self) -> None:
        payload = _session_authority_payload()
        payload["subject_ura"] = "easynet:///r/example/user/bob"
        value = _authority_metadata(payload, b"session-signature")

        with self.assertRaisesRegex(
            SDKError, "session authority user subject must match session_owner_user_id"
        ):
            SessionAuthority.from_metadata(value)

        payload = _session_authority_payload()
        payload["subject_ura"] = (
            "easynet:///r/example/resource/user.alice/session/session-2"
        )
        value = _authority_metadata(payload, b"session-signature")

        with self.assertRaisesRegex(
            SDKError,
            "session authority subject_ura owner/session must match session_owner_user_id and session_id",
        ):
            SessionAuthority.from_metadata(value)

        payload = _session_authority_payload()
        payload["session_id"] = "invocation_history"
        payload["subject_ura"] = (
            "easynet:///r/example/resource/user.alice/session/invocation_history"
        )
        value = _authority_metadata(payload, b"session-signature")

        with self.assertRaisesRegex(
            SDKError,
            "retired invocation-history subject",
        ):
            SessionAuthority.from_metadata(value)

        payload = _session_authority_payload()
        payload["session_owner_user_id"] = "teamalice"
        payload["subject_ura"] = (
            "easynet:///r/example/resource/user.team.alice/session/session-1"
        )
        value = _authority_metadata(payload, b"session-signature")

        with self.assertRaisesRegex(
            SDKError,
            "session authority subject_ura must be a canonical user or session subject",
        ):
            SessionAuthority.from_metadata(value)

        transport = _MemoryAuthorityTransport()
        client = AuthorityClient(transport)
        with self.assertRaisesRegex(
            SDKError,
            "session authority subject_ura must be a canonical user or session subject",
        ):
            client.mint_session_authority(
                SessionAuthorityRequest(
                    issuer_ura="easynet:///r/example/agent/backend",
                    session_id="session-1",
                    session_owner_user_id="alice",
                    creator_principal_id="easynet:///r/example/agent/backend",
                    callee_ura="easynet:///r/example/device/dev-a",
                    subject_ura="easynet:///r/example/device/dev-a",
                    audience="easynet:///r/example/device/dev-a",
                    scopes=("device.observe.*",),
                    allowed_actions=("read",),
                    allowed_followup_abilities=("device.observe.health",),
                    issued_at_ms=1000,
                    expires_at_ms=2000,
                )
            )
        self.assertEqual(transport.session_calls, 0)

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
                subject_ura="easynet:///r/example/resource/user.alice/session/session-1",
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

    def test_authority_client_projects_canonical_principal_uras_to_current_session_wire(self) -> None:
        payload = _session_authority_payload()
        payload["creator_principal_id"] = "easynet:///r/example/authority"
        value = _authority_metadata(payload, b"session-signature")
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
                session_owner_user_id="",
                creator_principal_id="",
                session_owner_ura="easynet:///r/example/user/alice",
                creator_principal_ura="easynet:///r/example/authority",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/resource/user.alice/session/session-1",
                audience="easynet:///r/example/device/dev-a",
                scopes=("device.observe.*",),
                allowed_actions=("read",),
                allowed_followup_abilities=("device.observe.health",),
                issued_at_ms=1000,
                expires_at_ms=2000,
            )
        )

        self.assertEqual(authority.session_owner_ura, "easynet:///r/example/user/alice")
        self.assertEqual(
            authority.creator_principal_ura, "easynet:///r/example/authority"
        )
        self.assertEqual(transport.seen_session["session_owner_user_id"], "alice")
        self.assertEqual(
            transport.seen_session["creator_principal_id"],
            "easynet:///r/example/authority",
        )
        self.assertNotIn("session_owner_ura", transport.seen_session)

    def test_session_authority_request_requires_explicit_creator_principal_ura(self) -> None:
        request = SessionAuthorityRequest(
            issuer_ura="easynet:///r/example/agent/backend",
            session_id="session-1",
            session_owner_user_id="alice",
            creator_principal_id="easynet:///r/example/authority",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/resource/user.alice/session/session-1",
            audience="easynet:///r/example/device/dev-a",
            scopes=("device.observe.*",),
            allowed_actions=("read",),
            allowed_followup_abilities=("device.observe.health",),
            issued_at_ms=1000,
            expires_at_ms=2000,
        )

        normalized = authority_module._normalized_session_authority_request(request)

        self.assertEqual(
            normalized.creator_principal_id, "easynet:///r/example/authority"
        )
        self.assertEqual(normalized.creator_principal_ura, "")

    def test_authority_client_rejects_conflicting_canonical_principal_uras(self) -> None:
        client = AuthorityClient(_MemoryAuthorityTransport())

        with self.assertRaises(SDKError) as caught:
            client.mint_session_authority(
                SessionAuthorityRequest(
                    issuer_ura="easynet:///r/example/agent/backend",
                    session_id="session-1",
                    session_owner_user_id="bob",
                    session_owner_ura="easynet:///r/example/user/alice",
                    creator_principal_id="easynet:///r/example/agent/backend",
                    callee_ura="easynet:///r/example/device/dev-a",
                    subject_ura="easynet:///r/example/resource/user.alice/session/session-1",
                    audience="easynet:///r/example/device/dev-a",
                    scopes=("device.observe.*",),
                    allowed_actions=("read",),
                    allowed_followup_abilities=("device.observe.health",),
                    issued_at_ms=1000,
                    expires_at_ms=2000,
                )
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn("session_owner_user_id must match", str(caught.exception))

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
                    subject_ura="easynet:///r/example/resource/user.alice/session/session-1",
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
        "subject_ura": "easynet:///r/example/resource/user.alice/session/session-1",
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


class _CanonicalSigner:
    def __init__(self) -> None:
        self.payloads: list[bytes] = []

    def sign_canonical(self, payload: bytes) -> bytes:
        self.payloads.append(payload)
        return b"s" * 64



if __name__ == "__main__":
    unittest.main()
