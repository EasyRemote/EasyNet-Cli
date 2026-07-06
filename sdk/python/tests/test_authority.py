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
    is_code,
)


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
        value = _authority_metadata(
            {
                "backend_ura": "easynet:///r/example/agent/backend",
                "user_ura": "easynet:///r/example/user/alice",
                "session_id": "sa-example",
                "scopes": ["device.observe.*"],
                "audiences": ["easynet:///r/example/device/dev-a"],
                "issued_at_ms": 1000,
                "expires_at_ms": 2000,
            },
            b"session-signature",
        )

        authority = SessionAuthority.from_metadata(value)

        self.assertEqual(authority.backend_ura, "easynet:///r/example/agent/backend")
        self.assertEqual(authority.session_id, "sa-example")
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
        value = _authority_metadata(
            {
                "backend_ura": "easynet:///r/example/agent/backend",
                "user_ura": "easynet:///r/example/user/alice",
                "session_id": "sa-example",
                "scopes": ["device.observe.*"],
                "audiences": ["easynet:///r/example/device/dev-a"],
                "issued_at_ms": 1000,
                "expires_at_ms": 2000,
            },
            b"session-signature",
        )
        transport = _MemoryAuthorityTransport(
            session_json=json.dumps(
                {"metadata": {SESSION_AUTHORITY_METADATA_KEY: value}}
            ).encode("utf-8")
        )
        client = AuthorityClient(transport)

        authority = client.mint_session_authority(
            SessionAuthorityRequest(
                backend_ura="easynet:///r/example/agent/backend",
                user_ura="easynet:///r/example/user/alice",
                session_id="sa-example",
                scopes=("device.observe.*",),
                audiences=("easynet:///r/example/device/dev-a",),
                issued_at_ms=1000,
                expires_at_ms=2000,
            )
        )

        self.assertEqual(authority.session_id, "sa-example")
        self.assertEqual(authority.metadata().value, value)
        self.assertEqual(transport.seen_session["session_id"], "sa-example")

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
                    backend_ura="easynet:///r/example/agent/backend",
                    user_ura="easynet:///r/example/user/alice",
                    session_id="sa-example",
                    scopes=("device.observe.*",),
                    audiences=("easynet:///r/example/device/dev-a",),
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


if __name__ == "__main__":
    unittest.main()
