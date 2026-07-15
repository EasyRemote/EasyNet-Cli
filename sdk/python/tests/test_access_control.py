import unittest

from easynet_sdk.access_control import (
    AccessControlCheckRequest,
    AccessControlEffect,
    AccessControlGrant,
    AccessControlGrantRequest,
    AccessControlGrantState,
    AccessControlAdmissionExplainRequest,
    AccessControlListRequest,
    AccessControlPermissionRequest,
    AccessControlPermissionRequestCreateRequest,
    AccessControlPermissionRequestListRequest,
    AccessControlPermissionRequestResolveRequest,
    AccessControlPrincipalKind,
    RuntimeAccessControlProvider,
)
from easynet_sdk.errors import ErrorCode, SDKError, is_code
from easynet_sdk.runtime_ability import RuntimeCallContext


class _MemoryAbility:
    def __init__(self) -> None:
        self.ability = ""
        self.arguments: dict[str, object] = {}

    def invoke(
        self, call: RuntimeCallContext, ability_name: str, arguments: object
    ) -> dict[str, object]:
        if not (
            call.caller_ura
            and call.callee_ura
            and call.subject_ura
            and call.nonce_base64
            and call.causal_context is not None
        ):
            raise AssertionError("call context was not preserved")
        self.ability = ability_name
        self.arguments = dict(arguments)  # type: ignore[arg-type]
        if ability_name == "authority.binding.grant":
            return {
                "grant": self.arguments["grant"],
                "idempotent_replay": True,
                "audit_record_id": "audit-1",
            }
        if ability_name == "authority.binding.list":
            return {
                "grants": [
                    {
                        "grant_id": "grant-1",
                        "owner_ura": "easynet:///r/example/user/alice",
                        "principal_kind": "user",
                        "principal_id": "bob",
                        "principal_ura": "easynet:///r/example/user/bob",
                        "token_class": "service",
                        "actions": ["invoke"],
                        "effect": "allow",
                        "lifetime": "session",
                        "state": "active",
                        "created_by": "easynet:///r/example/user/alice",
                        "updated_at": "2026-07-11T00:00:00Z",
                        "review_required_after": "2026-08-11T00:00:00Z",
                        "last_reviewed_at": "2026-07-10T00:00:00Z",
                        "last_used_at": "2026-07-11T01:00:00Z",
                        "reason": "operator-approved",
                    }
                ]
            }
        if ability_name == "authority.binding.check":
            return {
                "policy_decision": {
                    "decision": "allow",
                    "owner_ura": "easynet:///r/example/user/alice",
                    "principal_kind": "user",
                    "principal_ura": "easynet:///r/example/user/bob",
                    "action": "invoke",
                }
            }
        if ability_name == "policy.request.create":
            return {"request": self.arguments["request"]}
        if ability_name == "policy.request.resolve":
            return {
                "request": self.arguments["request"],
                "created_grant": self.arguments["created_grant"],
                "authority_proof": {
                    "proof_id": "proof-1",
                    "owner_ura": "easynet:///r/example/user/alice",
                    "principal_kind": "user",
                    "principal_id": "bob",
                    "principal_ura": "easynet:///r/example/user/bob",
                },
                "idempotent_replay": True,
            }
        if ability_name == "policy.request.list":
            return {
                "requests": [
                    {
                        "request_id": "request-1",
                        "owner_ura": "easynet:///r/example/user/alice",
                        "principal_kind": "user",
                        "principal_id": "bob",
                        "principal_ura": "easynet:///r/example/user/bob",
                        "callee_ura": "easynet:///r/example/device/dev-a",
                        "subject_ura": "easynet:///r/example/resource/user.alice/session/session-1",
                        "ability_ura": "easynet:///r/example/device/dev-a/ability/device.observe.health",
                        "action": "invoke",
                        "status": "pending",
                    }
                ]
            }
        if ability_name == "admission.explain":
            return {
                "observer_ura": "easynet:///r/example/user/alice",
                "redacted": True,
                "redaction_reason": "not_owner",
                "authority_reason": "observer redacted",
                "root_trace": {
                    "invocation_id": "inv-1",
                    "stage": "admission",
                    "redacted": True,
                },
                "policy_decision": {"decision": "deny", "reason": "not_owner"},
                "signature_decision": {"decision": "allow"},
            }
        raise AssertionError(f"unexpected ability {ability_name}")


class AccessControlTests(unittest.TestCase):
    def test_runtime_provider_grants_with_canonical_principal_uras(self) -> None:
        ability = _MemoryAbility()
        provider = RuntimeAccessControlProvider(ability)

        result = provider.grant(
            AccessControlGrantRequest(
                call=_call(),
                grant=AccessControlGrant(
                    grant_id="grant-1",
                    owner_ura="easynet:///r/example/user/alice",
                    principal_kind=AccessControlPrincipalKind.USER,
                    principal_ura="easynet:///r/example/user/bob",
                    token_class="service",
                    ability_ura_pattern="easynet:///r/example/device/dev-a/ability/device.observe.health",
                    actions=("invoke",),
                    lifetime="session",
                    created_by="easynet:///r/example/user/alice",
                    updated_at="2026-07-11T00:00:00Z",
                    last_used_at="2026-07-11T01:00:00Z",
                    reason="operator-approved",
                ),
            )
        )

        self.assertEqual(ability.ability, "authority.binding.grant")
        grant = ability.arguments["grant"]
        self.assertIsInstance(grant, dict)
        self.assertEqual(ability.arguments["owner_ura"], "easynet:///r/example/user/alice")
        self.assertEqual(ability.arguments["principal_ura"], "easynet:///r/example/user/bob")
        self.assertNotIn("owner_user_id", grant)
        self.assertNotIn("principal_id", grant)
        self.assertEqual(grant["token_class"], "service")  # type: ignore[index]
        self.assertEqual(grant["lifetime"], "session")  # type: ignore[index]
        self.assertEqual(grant["last_used_at"], "2026-07-11T01:00:00Z")  # type: ignore[index]
        self.assertEqual(grant["reason"], "operator-approved")  # type: ignore[index]
        self.assertEqual(result.grant.owner_ura, "easynet:///r/example/user/alice")
        self.assertEqual(result.grant.principal_ura, "easynet:///r/example/user/bob")
        self.assertTrue(result.idempotent_replay)
        self.assertNotIn("backend_account_id", ability.arguments)

    def test_runtime_provider_lists_and_checks_canonical_policy(self) -> None:
        ability = _MemoryAbility()
        provider = RuntimeAccessControlProvider(ability)

        page = provider.list(
            AccessControlListRequest(
                call=_call(),
                owner_ura="easynet:///r/example/user/alice",
                principal_kind=AccessControlPrincipalKind.USER,
                principal_ura="easynet:///r/example/user/bob",
                ability_ura="easynet:///r/example/device/dev-a/ability/device.observe.health",
                subject_ura="easynet:///r/example/resource/user.alice/session/session-1",
                action="invoke",
                effect=AccessControlEffect.ALLOW,
                state=AccessControlGrantState.ACTIVE,
                limit=10,
                cursor="cursor-1",
            )
        )

        self.assertEqual(len(page.grants), 1)
        self.assertEqual(page.grants[0].grant_id, "grant-1")
        self.assertEqual(ability.arguments["owner_ura"], "easynet:///r/example/user/alice")
        self.assertEqual(ability.arguments["principal_ura"], "easynet:///r/example/user/bob")
        self.assertNotIn("owner_user_id", ability.arguments)
        self.assertNotIn("principal_id", ability.arguments)
        self.assertEqual(ability.arguments["limit"], 10)
        self.assertEqual(ability.arguments["cursor"], "cursor-1")
        self.assertEqual(page.grants[0].token_class, "service")
        self.assertEqual(page.grants[0].lifetime, "session")
        self.assertEqual(page.grants[0].reason, "operator-approved")

        result = provider.check(
            AccessControlCheckRequest(
                call=_call(),
                owner_ura="easynet:///r/example/user/alice",
                principal_kind=AccessControlPrincipalKind.USER,
                principal_ura="easynet:///r/example/user/bob",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/resource/user.alice/session/session-1",
                ability_ura="easynet:///r/example/device/dev-a/ability/device.observe.health",
                action="invoke",
                safe_read=True,
            )
        )

        self.assertEqual(result.policy_decision.decision, "allow")

    def test_runtime_provider_rejects_non_user_owner_ura(self) -> None:
        provider = RuntimeAccessControlProvider(_MemoryAbility())

        with self.assertRaises(SDKError) as caught:
            provider.list(
                AccessControlListRequest(
                    call=_call(),
                    owner_ura="easynet:///r/example/device/dev-a",
                )
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_runtime_provider_manages_permission_requests(self) -> None:
        ability = _MemoryAbility()
        provider = RuntimeAccessControlProvider(ability)

        created = provider.create_request(
            AccessControlPermissionRequestCreateRequest(
                call=_call(),
                request=_permission_request(),
                actor_ura="easynet:///r/example/user/alice",
            )
        )

        self.assertEqual(created.request_id, "request-1")
        self.assertEqual(ability.ability, "policy.request.create")
        request_wire = ability.arguments["request"]
        self.assertIsInstance(request_wire, dict)
        self.assertNotIn("owner_user_id", request_wire)
        self.assertNotIn("principal_id", request_wire)
        self.assertEqual(ability.arguments["actor_ura"], "easynet:///r/example/user/alice")

        resolved = provider.resolve_request(
            AccessControlPermissionRequestResolveRequest(
                call=_call(),
                request=_permission_request(),
                created_grant=AccessControlGrant(
                    grant_id="grant-1",
                    owner_ura="easynet:///r/example/user/alice",
                    principal_kind=AccessControlPrincipalKind.USER,
                    principal_ura="easynet:///r/example/user/bob",
                    actions=("invoke",),
                    created_by="easynet:///r/example/user/alice",
                ),
                actor_ura="easynet:///r/example/user/alice",
            )
        )

        self.assertTrue(resolved.idempotent_replay)
        self.assertIsNotNone(resolved.created_grant)
        self.assertIsNotNone(resolved.authority_proof)

        listed = provider.list_requests(
            AccessControlPermissionRequestListRequest(
                call=_call(),
                owner_ura="easynet:///r/example/user/alice",
                principal_kind=AccessControlPrincipalKind.USER,
                principal_ura="easynet:///r/example/user/bob",
                status="pending",
                limit=10,
                cursor="cursor-1",
            )
        )

        self.assertEqual(len(listed.requests), 1)
        self.assertEqual(listed.requests[0].request_id, "request-1")
        self.assertEqual(ability.arguments["cursor"], "cursor-1")

    def test_runtime_provider_explains_admission(self) -> None:
        ability = _MemoryAbility()
        provider = RuntimeAccessControlProvider(ability)

        result = provider.explain(
            AccessControlAdmissionExplainRequest(
                call=_call(),
                observer_ura="easynet:///r/example/user/alice",
                invocation_id="inv-1",
            )
        )

        self.assertEqual(ability.ability, "admission.explain")
        self.assertTrue(result.redacted)
        self.assertIsNotNone(result.root_trace)
        self.assertIsNotNone(result.policy_decision)


def _call() -> RuntimeCallContext:
    return RuntimeCallContext(
        caller_ura="easynet:///r/example/user/alice",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/resource/user.alice/access-control",
        nonce_base64="bm9uY2U=",
        causal_context={"kind": "none"},
    )


def _permission_request() -> AccessControlPermissionRequest:
    return AccessControlPermissionRequest(
        request_id="request-1",
        owner_ura="easynet:///r/example/user/alice",
        principal_kind=AccessControlPrincipalKind.USER,
        principal_ura="easynet:///r/example/user/bob",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/resource/user.alice/session/session-1",
        ability_ura="easynet:///r/example/device/dev-a/ability/device.observe.health",
        action="invoke",
        requested_lifetimes=("session",),
        status="pending",
    )


if __name__ == "__main__":
    unittest.main()
