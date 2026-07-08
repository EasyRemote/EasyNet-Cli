import json
import pathlib
import unittest

from easynet_sdk import (
    AbilityCallTrace,
    AccessAction,
    AccessControlClient,
    AdmissionExplainResult,
    AuthorityProof,
    PermissionGrant,
    PermissionRequest,
    PolicyDecision,
    PrincipalKind,
    SignatureDecision,
)


FIXTURES = pathlib.Path(__file__).resolve().parents[2] / "conformance" / "fixtures"


class FakeAccessControlTransport:
    def __init__(self) -> None:
        self.last: dict[str, object] = {}

    def _decode(self, request_json: bytes) -> None:
        self.last = json.loads(request_json.decode("utf-8"))

    def grant_authority_binding(self, request_json: bytes) -> bytes:
        self._decode(request_json)
        return json.dumps(
            {
                "grant": {
                    "grant_id": "grant-1",
                    "owner_user_id": "alice",
                    "principal_kind": "token",
                    "principal_id": "token-principal",
                    "actions": ["read"],
                    "effect": "allow",
                    "lifetime": "permanent",
                    "state": "active",
                    "created_by": "easynet:///r/test/user/alice",
                    "created_at": "2026-07-09T00:00:00Z",
                },
                "idempotent_replay": False,
                "audit_record_id": "audit-1",
            }
        ).encode("utf-8")

    def revoke_authority_binding(self, request_json: bytes) -> bytes:
        self._decode(request_json)
        return json.dumps({"grant": self.last.get("grant", {})}).encode("utf-8")

    def list_authority_bindings(self, request_json: bytes) -> bytes:
        self._decode(request_json)
        return b'{"grants":[]}'

    def check_authority_binding(self, request_json: bytes) -> bytes:
        self._decode(request_json)
        return b'{"policy_decision":{"decision":"deny","reason":"NON_INTERACTIVE_DENY"}}'

    def create_policy_request(self, request_json: bytes) -> bytes:
        self._decode(request_json)
        return json.dumps({"request": self.last["request"]}).encode("utf-8")

    def resolve_policy_request(self, request_json: bytes) -> bytes:
        return self.create_policy_request(request_json)

    def list_policy_requests(self, request_json: bytes) -> bytes:
        self._decode(request_json)
        return b'{"requests":[]}'

    def explain_admission(self, request_json: bytes) -> bytes:
        self._decode(request_json)
        return b'{"observer_ura":"easynet:///r/test/user/alice","redacted":true,"authority_reason":"AUTHORITY_PROOF_MISSING"}'


class AccessControlTests(unittest.TestCase):
    def test_grant_uses_typed_transport(self) -> None:
        transport = FakeAccessControlTransport()
        client = AccessControlClient(transport)
        result = client.grant(
            PermissionGrant(
                grant_id="grant-1",
                owner_user_id="alice",
                principal_kind=PrincipalKind.TOKEN,
                principal_id="token-principal",
                actions=(AccessAction.READ,),
                effect="allow",
                lifetime="permanent",
                state="active",
                created_by="easynet:///r/test/user/alice",
                created_at="2026-07-09T00:00:00Z",
            ),
            actor_ura="easynet:///r/test/user/alice",
        )

        self.assertEqual(result.grant.grant_id, "grant-1")
        self.assertEqual(result.audit_record_id, "audit-1")
        self.assertEqual(transport.last["actor_ura"], "easynet:///r/test/user/alice")

    def test_explain_projects_rfc014_dto(self) -> None:
        client = AccessControlClient(FakeAccessControlTransport())
        result = client.explain({"observer_ura": "easynet:///r/test/user/alice"})

        self.assertIsInstance(result, AdmissionExplainResult)
        self.assertTrue(result.redacted)
        self.assertEqual(result.raw["authority_reason"], "AUTHORITY_PROOF_MISSING")

    def test_shared_rfc014_fixtures_decode(self) -> None:
        grant = PermissionGrant.from_dict(_fixture("access-control-permission-grant.v4.json"))
        request = PermissionRequest.from_dict(_fixture("access-control-permission-request.v4.json"))
        proof = AuthorityProof.from_dict(_fixture("access-control-authority-proof.v4.json"))
        policy = PolicyDecision(_fixture("access-control-policy-decision.v4.json"))
        signature = SignatureDecision(_fixture("access-control-signature-decision.v4.json"))
        trace = AbilityCallTrace(_fixture("access-control-ability-call-trace.v4.json"))
        explain = AdmissionExplainResult.from_dict(
            _fixture("access-control-admission-explain-result.v4.json")
        )

        self.assertEqual(grant.grant_id, "grant-0001")
        self.assertEqual(request.request_id, "req-0001")
        self.assertEqual(proof.proof_id, "proof-0001")
        self.assertEqual(proof.raw["audience_ura"], "easynet:///r/example/device/dev-a")
        self.assertEqual(policy.reason, "TOKEN_SCOPE_DENIED")
        self.assertEqual(signature.reason, "CALLER_KEY_NOT_FOUND")
        self.assertEqual(trace.invocation_id, "inv-0001")
        self.assertFalse(explain.redacted)


def _fixture(name: str) -> dict[str, object]:
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
