from dataclasses import FrozenInstanceError

import pytest

import easynet_sdk.principal as principal_module
from easynet_sdk.principal import (
    PrincipalCommand,
    PrincipalProofKind,
    PrincipalProofRef,
    PrincipalState,
    PublicKeyBinding,
    PublicKeyBindingState,
    grant_actions,
)


def test_principal_state_vocabulary_matches_go_contract() -> None:
    assert [state.value for state in PrincipalState] == [
        "pending",
        "active",
        "suspended",
        "deleted",
    ]
    assert [state.value for state in PublicKeyBindingState] == [
        "active",
        "rotated",
        "revoked",
    ]


def test_principal_projections_are_immutable_and_public_only() -> None:
    command = PrincipalCommand(
        actor_ura="easynet:///r/example/user/admin",
        idempotency_key="attempt-1",
        proof=PrincipalProofRef(PrincipalProofKind.BOOTSTRAP, "proof-1"),
    )
    with pytest.raises(FrozenInstanceError):
        command.actor_ura = "changed"  # type: ignore[misc]

    fields = PublicKeyBinding.__dataclass_fields__
    assert "private_key" not in fields
    assert "seed" not in fields
    assert "vault" not in fields


def test_grant_actions_returns_immutable_projection() -> None:
    source = ["principal.key.add", "principal.key.revoke"]
    projected = grant_actions(source)
    source.append("principal.delete")
    assert projected == ("principal.key.add", "principal.key.revoke")


def test_principal_module_does_not_export_implementation_dependencies() -> None:
    assert "dataclass" not in principal_module.__all__
    assert "Protocol" not in principal_module.__all__
    assert "Sequence" not in principal_module.__all__
