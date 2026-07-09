import json
import pathlib
import unittest

from easynet_sdk import (
    AbilityCallTrace,
    AccessAction,
    AccessControlClient,
    AccessControlCarrierBase,
    AdmissionExplainResult,
    AdmissionExplainRequest,
    AuthorityProof,
    AuthorityBindingCheckRequest,
    PermissionGrant,
    PermissionRequest,
    PermissionRequestResolutionResult,
    PolicyRequestCreateRequest,
    PolicyRequestListRequest,
    PolicyRequestResolveRequest,
    PolicyDecision,
    PrincipalKind,
    RuntimeAccessControlTransport,
    RuntimeClient,
    IdentityClient,
    SDKError,
    ErrorCode,
    SignatureDecision,
    is_code,
)
from easynet_sdk.access_control import (
    AuthorityBindingGrantRequest,
    AuthorityBindingListRequest,
    AuthorityBindingRevokeRequest,
)
from easynet_sdk.system_abilities import AccessControlSystemAbility
from test_runtime import MemoryRuntimeTransport


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
        self._decode(request_json)
        return json.dumps(
            {
                "request": {
                    "request_id": "req-1",
                    "owner_user_id": "alice",
                    "caller_ura": "c",
                    "principal_kind": "token",
                    "principal_id": "p",
                    "callee_ura": "d",
                    "subject_ura": "s",
                    "ability_ura": "terminal.create",
                    "action": "stream",
                    "requested_lifetimes": ["session"],
                    "status": "approved",
                    "created_at": "t",
                    "expires_at": "e",
                    "created_grant_id": "grant-approval-1",
                },
                "created_grant": {
                    "grant_id": "grant-approval-1",
                    "owner_user_id": "alice",
                    "principal_kind": "token",
                    "principal_id": "p",
                    "callee_ura": "d",
                    "subject_ura_pattern": "s",
                    "ability_ura_pattern": "terminal.create",
                    "actions": ["stream"],
                    "effect": "allow",
                    "lifetime": "session",
                    "state": "active",
                    "created_by": "easynet:///r/test/user/alice",
                    "created_at": "t",
                },
                "idempotent_replay": False,
            }
        ).encode("utf-8")

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

    def test_resolve_request_with_grant_uses_typed_resolution(self) -> None:
        transport = FakeAccessControlTransport()
        client = AccessControlClient(transport)
        request = PermissionRequest.from_dict(
            {
                "request_id": "req-1",
                "owner_user_id": "alice",
                "caller_ura": "c",
                "principal_kind": "token",
                "principal_id": "p",
                "callee_ura": "d",
                "subject_ura": "s",
                "ability_ura": "terminal.create",
                "action": "stream",
                "requested_lifetimes": ["session"],
                "status": "approved",
                "created_at": "t",
                "expires_at": "e",
            }
        )
        grant = PermissionGrant(
            grant_id="grant-approval-1",
            owner_user_id="alice",
            principal_kind=PrincipalKind.TOKEN,
            principal_id="p",
            callee_ura="d",
            subject_ura_pattern="s",
            ability_ura_pattern="terminal.create",
            actions=(AccessAction.STREAM,),
            effect="allow",
            lifetime="session",
            state="active",
            created_by="easynet:///r/test/user/alice",
            created_at="t",
        )

        result = client.resolve_request_with_grant(
            request,
            grant,
            actor_ura="easynet:///r/test/user/alice",
        )

        self.assertIsInstance(result, PermissionRequestResolutionResult)
        self.assertEqual(result.request.raw["created_grant_id"], "grant-approval-1")
        self.assertIsNotNone(result.created_grant)
        self.assertEqual(result.created_grant.grant_id, "grant-approval-1")
        self.assertIn("created_grant", transport.last)

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

    def test_runtime_transport_builds_authority_grant_invocation(self) -> None:
        runtime_transport = AccessControlRuntimeTransport(output_json=_grant_output())
        client = AccessControlClient(
            RuntimeAccessControlTransport(
                runtime=RuntimeClient(runtime_transport),
                identity=IdentityClient(AccessControlIdentityTransport()),
            )
        )

        result = client.grant_with_request(
            AuthorityBindingGrantRequest(
                carrier=_carrier(),
                grant=_grant(),
                actor_ura="easynet:///r/example/user/alice",
            )
        )

        self.assertEqual(result.grant.grant_id, "grant-1")
        draft = runtime_transport.seen_draft
        self.assertIsNotNone(draft)
        assert draft is not None
        self.assertEqual(
            draft["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.authority.binding.grant@1.0.0",
        )
        self.assertEqual(
            draft["metadata"]["system_ability"],
            AccessControlSystemAbility.AUTHORITY_BINDING_GRANT.value,
        )
        self.assertEqual(draft["metadata"]["carrier_owner"], "daemon_sdk")
        self.assertNotIn("carrier", draft["args"])
        self.assertEqual(draft["args"]["actor_ura"], "easynet:///r/example/user/alice")

    def test_runtime_transport_dispatches_rfc014_ability_matrix(self) -> None:
        runtime_transport = AccessControlRuntimeTransport(output_json={})
        transport = RuntimeAccessControlTransport(
            runtime=RuntimeClient(runtime_transport),
            identity=IdentityClient(AccessControlIdentityTransport()),
        )
        base = _carrier()
        cases = (
            (
                AccessControlSystemAbility.AUTHORITY_BINDING_GRANT,
                transport.grant_authority_binding,
                AuthorityBindingGrantRequest(
                    carrier=base,
                    grant=_grant(),
                    actor_ura="easynet:///r/example/user/alice",
                ).to_json_dict(),
            ),
            (
                AccessControlSystemAbility.AUTHORITY_BINDING_REVOKE,
                transport.revoke_authority_binding,
                AuthorityBindingRevokeRequest(
                    carrier=base,
                    owner_user_id="alice",
                    grant_id="grant-1",
                    actor_ura="easynet:///r/example/user/alice",
                ).to_json_dict(),
            ),
            (
                AccessControlSystemAbility.AUTHORITY_BINDING_LIST,
                transport.list_authority_bindings,
                AuthorityBindingListRequest(carrier=base, owner_user_id="alice").to_json_dict(),
            ),
            (
                AccessControlSystemAbility.AUTHORITY_BINDING_CHECK,
                transport.check_authority_binding,
                AuthorityBindingCheckRequest(
                    carrier=base,
                    caller_ura=base.caller_ura,
                    principal_kind=PrincipalKind.TOKEN,
                    principal_id="token-principal",
                    callee_ura=base.callee_ura,
                    subject_ura=base.subject_ura,
                    ability_ura="easynet:///r/example/ability/device.dev-a.terminal.create",
                    action=AccessAction.STREAM,
                ).to_json_dict(),
            ),
            (
                AccessControlSystemAbility.POLICY_REQUEST_CREATE,
                transport.create_policy_request,
                PolicyRequestCreateRequest(
                    carrier=base,
                    request=_permission_request(),
                    actor_ura="easynet:///r/example/user/alice",
                ).to_json_dict(),
            ),
            (
                AccessControlSystemAbility.POLICY_REQUEST_RESOLVE,
                transport.resolve_policy_request,
                PolicyRequestResolveRequest(
                    carrier=base,
                    request=_permission_request(),
                    actor_ura="easynet:///r/example/user/alice",
                ).to_json_dict(),
            ),
            (
                AccessControlSystemAbility.POLICY_REQUEST_LIST,
                transport.list_policy_requests,
                PolicyRequestListRequest(carrier=base, owner_user_id="alice").to_json_dict(),
            ),
            (
                AccessControlSystemAbility.ADMISSION_EXPLAIN,
                transport.explain_admission,
                AdmissionExplainRequest(
                    carrier=base,
                    observer_ura="easynet:///r/example/user/alice",
                    trace_id="trace-1",
                ).to_json_dict(),
            ),
        )
        for ability, call, payload in cases:
            runtime_transport.output_json = _output_for_ability(ability)
            call(json.dumps(payload).encode("utf-8"))
            assert runtime_transport.seen_draft is not None
            self.assertEqual(
                runtime_transport.seen_draft["metadata"]["system_ability"],
                ability.value,
            )

    def test_runtime_transport_requires_carrier(self) -> None:
        client = AccessControlClient(
            RuntimeAccessControlTransport(
                runtime=RuntimeClient(AccessControlRuntimeTransport(output_json={"grants": []})),
                identity=IdentityClient(AccessControlIdentityTransport()),
            )
        )
        with self.assertRaises(SDKError) as caught:
            client.list_grants({"owner_user_id": "alice"})
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))


def _fixture(name: str) -> dict[str, object]:
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


class AccessControlRuntimeTransport(MemoryRuntimeTransport):
    def __init__(self, *, output_json: dict[str, object]) -> None:
        super().__init__()
        self.output_json = output_json

    def invoke(self, draft_json: bytes) -> bytes:
        self.seen_draft = json.loads(draft_json.decode("utf-8"))
        return json.dumps(
            {
                "ok": True,
                "tuple": self.seen_draft,
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_json": self.output_json,
                "elapsed_ms": 7,
                "receipt": {"receipt_id": "access-control-runtime-1"},
                "error": None,
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")


class AccessControlIdentityTransport:
    def project_descriptor_ref(self, request_json: bytes) -> bytes:
        raise AssertionError("project_descriptor_ref should not be called")

    def build_descriptor_ref(self, request_json: bytes) -> bytes:
        request = json.loads(request_json.decode("utf-8"))
        ability_ura = request["ability_ura"]
        return json.dumps(
            {
                "kind": "descriptor_ref",
                "valid": True,
                "descriptor_ref": f"{ability_ura}@{request['descriptor_version']}",
                "ability_ura": ability_ura,
                "descriptor_version": request["descriptor_version"],
                "profile": "easynet-strict-v2",
                "components": {"owner_ura": "easynet:///r/example/device/dev-a"},
                "metadata": {"grammar_owner": "axon"},
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")

    def project_identity(self, request_json: bytes) -> bytes:
        request = json.loads(request_json.decode("utf-8"))
        ura = request["ura"]
        kind = "device" if "/device/" in ura else "resource"
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
            sort_keys=True,
        ).encode("utf-8")

    def build_ura(self, request_json: bytes) -> bytes:
        request = json.loads(request_json.decode("utf-8"))
        if request.get("kind") != "ability":
            raise AssertionError(f"unexpected URA build request: {request}")
        ability_ura = "easynet:///r/example/ability/device.dev-a." + str(
            request["ability_name"]
        )
        return json.dumps(
            {
                "kind": "ability",
                "valid": True,
                "ura": ability_ura,
                "profile": "easynet-strict-v2",
                "components": {"owner_ura": request["owner_ura"]},
                "metadata": {"grammar_owner": "axon"},
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")

    def build_resource_ref(self, request_json: bytes) -> bytes:
        raise AssertionError("build_resource_ref should not be called")

    def build_register_signing_key_invocation(self, request_json: bytes) -> bytes:
        raise AssertionError("build_register_signing_key_invocation should not be called")

    def build_list_signing_keys_invocation(self, request_json: bytes) -> bytes:
        raise AssertionError("build_list_signing_keys_invocation should not be called")

    def build_revoke_signing_key_invocation(self, request_json: bytes) -> bytes:
        raise AssertionError("build_revoke_signing_key_invocation should not be called")

    def register_signing_key(self, request_json: bytes) -> bytes:
        raise AssertionError("register_signing_key should not be called")

    def list_signing_keys(self, request_json: bytes) -> bytes:
        raise AssertionError("list_signing_keys should not be called")

    def revoke_signing_key(self, request_json: bytes) -> bytes:
        raise AssertionError("revoke_signing_key should not be called")

    def signer(self, request_json: bytes) -> bytes:
        raise AssertionError("signer should not be called")


def _carrier() -> AccessControlCarrierBase:
    return AccessControlCarrierBase(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/device/dev-a",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        metadata={"request_id": "access-control-1"},
    )


def _grant() -> PermissionGrant:
    return PermissionGrant(
        grant_id="grant-1",
        owner_user_id="alice",
        principal_kind=PrincipalKind.TOKEN,
        principal_id="token-principal",
        actions=(AccessAction.READ,),
        effect="allow",
        lifetime="permanent",
        state="active",
        created_by="easynet:///r/example/user/alice",
        created_at="2026-07-09T00:00:00Z",
    )


def _permission_request() -> PermissionRequest:
    return PermissionRequest.from_dict(
        {
            "request_id": "req-1",
            "owner_user_id": "alice",
            "caller_ura": "c",
            "principal_kind": "token",
            "principal_id": "p",
            "callee_ura": "d",
            "subject_ura": "s",
            "ability_ura": "a",
            "action": "stream",
            "requested_lifetimes": ["session"],
            "status": "pending",
            "created_at": "t",
            "expires_at": "e",
        }
    )


def _grant_output() -> dict[str, object]:
    return {
        "grant": _grant().to_dict(),
        "idempotent_replay": False,
        "audit_record_id": "audit-1",
    }


def _output_for_ability(ability: AccessControlSystemAbility) -> dict[str, object]:
    if ability == AccessControlSystemAbility.AUTHORITY_BINDING_GRANT:
        return _grant_output()
    if ability == AccessControlSystemAbility.AUTHORITY_BINDING_REVOKE:
        grant = _grant().to_dict()
        grant["state"] = "revoked"
        return {"grant": grant}
    if ability == AccessControlSystemAbility.AUTHORITY_BINDING_LIST:
        return {"grants": []}
    if ability == AccessControlSystemAbility.AUTHORITY_BINDING_CHECK:
        return {"policy_decision": {"decision": "deny", "reason": "NON_INTERACTIVE_DENY"}}
    if ability == AccessControlSystemAbility.POLICY_REQUEST_CREATE:
        return {"request": _permission_request().to_dict()}
    if ability == AccessControlSystemAbility.POLICY_REQUEST_RESOLVE:
        request = _permission_request().to_dict()
        request["status"] = "approved"
        return {"request": request}
    if ability == AccessControlSystemAbility.POLICY_REQUEST_LIST:
        return {"requests": []}
    if ability == AccessControlSystemAbility.ADMISSION_EXPLAIN:
        return {
            "observer_ura": "easynet:///r/example/user/alice",
            "redacted": True,
            "authority_reason": "AUTHORITY_PROOF_MISSING",
        }
    return {}


if __name__ == "__main__":
    unittest.main()
