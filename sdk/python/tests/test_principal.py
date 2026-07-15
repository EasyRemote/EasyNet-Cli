import hashlib
from dataclasses import FrozenInstanceError
from pathlib import Path
import unittest

import easynet_sdk.principal as principal_module
from easynet_sdk._principal_routes import _PRINCIPAL_ROUTE_MANIFEST_SHA256
from easynet_sdk.principal import (
    BindPrincipalKeyRequest,
    IssueEnrollmentRequest,
    PrincipalClient,
    PrincipalCommand,
    PrincipalProofKind,
    PrincipalProofRef,
    PrincipalState,
    PublicKeyBinding,
    PublicKeyBindingState,
    RevokeEnrollmentRequest,
    RuntimePrincipalProvider,
    grant_actions,
)
from easynet_sdk.runtime_ability import RuntimeCallContext


class PrincipalTests(unittest.TestCase):
    def test_principal_routes_are_generated_from_manifest(self) -> None:
        manifest = (
            Path(__file__).resolve().parents[2]
            .parent
            / "provider_routes"
            / "easynet-principal-lifecycle-routes.v1.json"
        )
        digest = hashlib.sha256(manifest.read_bytes()).hexdigest()
        self.assertEqual(_PRINCIPAL_ROUTE_MANIFEST_SHA256, digest)

    def test_principal_state_vocabulary_matches_go_contract(self) -> None:
        self.assertEqual(
            [state.value for state in PrincipalState],
            ["pending", "active", "suspended", "deleted"],
        )
        self.assertEqual(
            [state.value for state in PublicKeyBindingState],
            ["active", "rotated", "revoked"],
        )

    def test_principal_projections_are_immutable_and_public_only(self) -> None:
        command = PrincipalCommand(
            actor_ura="easynet:///r/example/user/admin",
            idempotency_key="attempt-1",
            proof=PrincipalProofRef(PrincipalProofKind.BOOTSTRAP, "proof-1"),
        )
        with self.assertRaises(FrozenInstanceError):
            command.actor_ura = "changed"  # type: ignore[misc]

        fields = PublicKeyBinding.__dataclass_fields__
        self.assertNotIn("private_key", fields)
        self.assertNotIn("seed", fields)
        self.assertNotIn("vault", fields)

    def test_grant_actions_returns_immutable_projection(self) -> None:
        source = ["principal.key.add", "principal.key.revoke"]
        projected = grant_actions(source)
        source.append("principal.delete")
        self.assertEqual(projected, ("principal.key.add", "principal.key.revoke"))

    def test_principal_module_does_not_export_implementation_dependencies(self) -> None:
        self.assertNotIn("dataclass", principal_module.__all__)
        self.assertNotIn("Protocol", principal_module.__all__)
        self.assertNotIn("Sequence", principal_module.__all__)

    def test_runtime_principal_provider_lowers_lifecycle_transitions(self) -> None:
        ability = _MemoryAbility()
        provider = RuntimePrincipalProvider(ability, _call())
        client = PrincipalClient(provider)

        result = client.bind_first_key(
            BindPrincipalKeyRequest(
                command=_command(),
                principal_ura="easynet:///r/example/user/alice",
                public_key=bytes(32),
                key_id="laptop",
            )
        )

        self.assertEqual(ability.ability, "principal.lifecycle.bind_first_key")
        request = ability.arguments["request"]
        self.assertIsInstance(request, dict)
        command = request["command"]  # type: ignore[index]
        self.assertIsInstance(command, dict)
        proof = command["proof"]  # type: ignore[index]
        self.assertIsInstance(proof, dict)
        self.assertEqual(request["principal_ura"], "easynet:///r/example/user/alice")  # type: ignore[index]
        self.assertTrue(request["public_key_b64"])  # type: ignore[index]
        self.assertEqual(command["actor_ura"], "easynet:///r/example/user/admin")  # type: ignore[index]
        self.assertEqual(command["idempotency_key"], "idem-1")  # type: ignore[index]
        self.assertEqual(proof["kind"], "bootstrap")  # type: ignore[index]
        self.assertEqual(result.principal_ura, "easynet:///r/example/user/alice")
        self.assertIs(result.state, PrincipalState.ACTIVE)
        self.assertEqual(len(result.bindings), 1)
        self.assertIsNotNone(result.enrollment_proof)
        self.assertEqual(result.enrollment_proof.kind, PrincipalProofKind.BOOTSTRAP)  # type: ignore[union-attr]
        self.assertEqual(result.enrollment_proof.reference, "proof-1")  # type: ignore[union-attr]
        self.assertIsNotNone(result.recovery)
        self.assertEqual(len(result.enrollments), 1)
        self.assertEqual(result.enrollments[0].enrollment_id, "enroll-1")
        self.assertEqual(
            result.enrollments[0].consumed_by_principal_ura,
            "easynet:///r/example/user/bob",
        )
        self.assertEqual(len(result.grants), 1)
        self.assertNotIn("private_key", request)

    def test_runtime_principal_provider_lowers_enrollment_authority(self) -> None:
        ability = _MemoryAbility()
        provider = RuntimePrincipalProvider(ability, _call())

        provider.issue_enrollment(
            IssueEnrollmentRequest(
                command=_command(),
                principal_ura="easynet:///r/example/user/alice",
                subject_principal_ura="easynet:///r/example/user/bob",
            )
        )

        self.assertEqual(ability.ability, "principal.lifecycle.issue_enrollment")
        request = ability.arguments["request"]
        self.assertIsInstance(request, dict)
        self.assertEqual(request["principal_ura"], "easynet:///r/example/user/alice")  # type: ignore[index]
        self.assertEqual(request["subject_principal_ura"], "easynet:///r/example/user/bob")  # type: ignore[index]

        provider.revoke_enrollment(
            RevokeEnrollmentRequest(
                command=_command(),
                principal_ura="easynet:///r/example/user/alice",
                enrollment_id="enroll-1",
            )
        )

        self.assertEqual(ability.ability, "principal.lifecycle.revoke_enrollment")
        request = ability.arguments["request"]
        self.assertIsInstance(request, dict)
        self.assertEqual(request["enrollment_id"], "enroll-1")  # type: ignore[index]

    def test_runtime_principal_provider_uses_generic_get_ability(self) -> None:
        ability = _MemoryAbility()
        provider = RuntimePrincipalProvider(ability, _call())

        provider.get("easynet:///r/example/user/alice")

        self.assertEqual(ability.ability, "principal.lifecycle.get")
        self.assertEqual(
            ability.arguments["principal_ura"], "easynet:///r/example/user/alice"
        )

    def test_runtime_principal_provider_rejects_private_projection_fields(self) -> None:
        tests = {
            "top-level private seed": lambda principal: principal.update(
                {"private_key_seed": "forbidden"}
            ),
            "binding vault material": lambda principal: principal["bindings"][0].update(  # type: ignore[index,union-attr]
                {"vault_ciphertext": "forbidden"}
            ),
            "recovery master key": lambda principal: principal["recovery"].update(  # type: ignore[union-attr]
                {"master_key": "forbidden"}
            ),
            "enrollment keyring path": lambda principal: principal["enrollments"][0].update(  # type: ignore[index,union-attr]
                {"keyring_storage_path": "/tmp/forbidden"}
            ),
            "grant passphrase": lambda principal: principal["grants"][0].update(  # type: ignore[index,union-attr]
                {"passphrase": "forbidden"}
            ),
        }
        for name, mutate in tests.items():
            with self.subTest(name=name):
                provider = RuntimePrincipalProvider(
                    _PrivateProjectionAbility(mutate), _call()
                )
                with self.assertRaisesRegex(Exception, "forbidden private field"):
                    provider.get("easynet:///r/example/user/alice")


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
        return {
            "principal": {
                "principal_ura": "easynet:///r/example/user/alice",
                "state": "active",
                "version": 2,
                "created_unix_ms": 1_700_000_000_000,
                "updated_unix_ms": 1_700_000_001_000,
                "bindings": [
                    {
                        "binding_id": "binding-1",
                        "principal_ura": "easynet:///r/example/user/alice",
                        "key_id": "laptop",
                        "public_key_b64": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                        "state": "active",
                        "created_unix_ms": 1_700_000_000_000,
                    }
                ],
                "enrollment_proof": {
                    "kind": "bootstrap",
                    "reference": "proof-1",
                },
                "recovery": {
                    "policy_ref": "recovery-policy-1",
                    "enabled": True,
                    "updated_unix_ms": 1_700_000_001_000,
                },
                "enrollments": [
                    {
                        "enrollment_id": "enroll-1",
                        "issuer_ura": "easynet:///r/example/user/alice",
                        "subject_principal_ura": "easynet:///r/example/user/bob",
                        "created_unix_ms": 1_700_000_001_000,
                        "consumed_by_principal_ura": "easynet:///r/example/user/bob",
                        "consumed_unix_ms": 1_700_000_002_000,
                    }
                ],
                "grants": [
                    {
                        "grant_id": "grant-1",
                        "principal_ura": "easynet:///r/example/user/alice",
                        "issuer_ura": "easynet:///r/example/user/admin",
                        "actions": ["principal.key.add"],
                        "created_unix_ms": 1_700_000_001_000,
                    }
                ],
            }
        }


class _PrivateProjectionAbility(_MemoryAbility):
    def __init__(self, mutate) -> None:  # type: ignore[no-untyped-def]
        super().__init__()
        self._mutate = mutate

    def invoke(
        self, call: RuntimeCallContext, ability_name: str, arguments: object
    ) -> dict[str, object]:
        output = super().invoke(call, ability_name, arguments)
        principal = output["principal"]
        assert isinstance(principal, dict)
        self._mutate(principal)
        return output


def _command() -> PrincipalCommand:
    return PrincipalCommand(
        actor_ura="easynet:///r/example/user/admin",
        idempotency_key="idem-1",
        expected_version=1,
        proof=PrincipalProofRef(PrincipalProofKind.BOOTSTRAP, "proof-1"),
    )


def _call() -> RuntimeCallContext:
    return RuntimeCallContext(
        caller_ura="easynet:///r/example/user/admin",
        callee_ura="easynet:///r/example/hub",
        subject_ura="easynet:///r/example/user/alice",
        nonce_base64="bm9uY2U=",
        causal_context={"kind": "none"},
    )


if __name__ == "__main__":
    unittest.main()
