import unittest

from easynet_sdk.access_control import (
    AccessControlCheckRequest,
    AccessControlEffect,
    AccessControlGrant,
    AccessControlGrantRequest,
    AccessControlGrantState,
    AccessControlListRequest,
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
            return {"grant": self.arguments["grant"], "audit_record_id": "audit-1"}
        if ability_name == "authority.binding.list":
            return {
                "grants": [
                    {
                        "grant_id": "grant-1",
                        "owner_ura": "easynet:///r/example/user/alice",
                        "principal_kind": "user",
                        "principal_ura": "easynet:///r/example/user/bob",
                        "actions": ["invoke"],
                        "effect": "allow",
                        "state": "active",
                        "created_by": "easynet:///r/example/user/alice",
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
                    ability_ura_pattern="easynet:///r/example/device/dev-a/ability/device.observe.health",
                    actions=("invoke",),
                    created_by="easynet:///r/example/user/alice",
                ),
            )
        )

        self.assertEqual(ability.ability, "authority.binding.grant")
        grant = ability.arguments["grant"]
        self.assertIsInstance(grant, dict)
        self.assertEqual(ability.arguments["owner_ura"], "easynet:///r/example/user/alice")
        self.assertEqual(ability.arguments["principal_ura"], "easynet:///r/example/user/bob")
        self.assertEqual(grant["owner_user_id"], "alice")  # type: ignore[index]
        self.assertEqual(grant["principal_id"], "bob")  # type: ignore[index]
        self.assertEqual(result.grant.owner_ura, "easynet:///r/example/user/alice")
        self.assertEqual(result.grant.principal_ura, "easynet:///r/example/user/bob")
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
                action="invoke",
                effect=AccessControlEffect.ALLOW,
                state=AccessControlGrantState.ACTIVE,
                limit=10,
            )
        )

        self.assertEqual(len(page.grants), 1)
        self.assertEqual(page.grants[0].grant_id, "grant-1")
        self.assertEqual(ability.arguments["owner_user_id"], "alice")
        self.assertEqual(ability.arguments["principal_id"], "bob")
        self.assertEqual(ability.arguments["limit"], 10)

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


def _call() -> RuntimeCallContext:
    return RuntimeCallContext(
        caller_ura="easynet:///r/example/user/alice",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/resource/user.alice/access-control",
        nonce_base64="bm9uY2U=",
        causal_context={"kind": "none"},
    )


if __name__ == "__main__":
    unittest.main()
