"""Product-neutral principal lifecycle contracts.

The module exposes immutable public projections and a provider protocol. It
does not own account login, private-key custody, or a product-specific user
directory.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Protocol, Sequence

__all__ = [
    "AuthorizationGrant",
    "BindPrincipalKeyRequest",
    "ChangePrincipalStateRequest",
    "ConfigureRecoveryRequest",
    "CreatePrincipalRequest",
    "IssueGrantRequest",
    "PrincipalCommand",
    "PrincipalLifecycle",
    "PrincipalProofKind",
    "PrincipalProofRef",
    "PrincipalSnapshot",
    "PrincipalState",
    "PublicKeyBinding",
    "PublicKeyBindingState",
    "RecoverPrincipalRequest",
    "RecoveryPolicy",
    "RevokeGrantRequest",
    "RevokePrincipalKeyRequest",
    "RotatePrincipalKeyRequest",
    "grant_actions",
]


class PrincipalState(str, Enum):
    PENDING = "pending"
    ACTIVE = "active"
    SUSPENDED = "suspended"
    DELETED = "deleted"


class PublicKeyBindingState(str, Enum):
    ACTIVE = "active"
    ROTATED = "rotated"
    REVOKED = "revoked"


class PrincipalProofKind(str, Enum):
    BOOTSTRAP = "bootstrap"
    ACTIVE_KEY = "active_key"
    GRANT = "grant"
    ENROLLMENT = "enrollment"
    RECOVERY = "recovery"


@dataclass(frozen=True)
class PrincipalProofRef:
    kind: PrincipalProofKind
    reference: str


@dataclass(frozen=True)
class PrincipalCommand:
    actor_ura: str
    idempotency_key: str
    proof: PrincipalProofRef
    expected_version: int | None = None


@dataclass(frozen=True)
class PublicKeyBinding:
    binding_id: str
    principal_ura: str
    public_key: bytes
    state: PublicKeyBindingState
    created_unix_ms: int
    key_id: str = ""
    expires_unix_ms: int | None = None
    rotated_unix_ms: int | None = None
    revoked_unix_ms: int | None = None
    rotated_to: str = ""


@dataclass(frozen=True)
class RecoveryPolicy:
    policy_ref: str
    enabled: bool
    updated_unix_ms: int


@dataclass(frozen=True)
class AuthorizationGrant:
    grant_id: str
    principal_ura: str
    issuer_ura: str
    actions: tuple[str, ...]
    created_unix_ms: int
    expires_unix_ms: int | None = None
    revoked_unix_ms: int | None = None


@dataclass(frozen=True)
class PrincipalSnapshot:
    principal_ura: str
    state: PrincipalState
    version: int
    bindings: tuple[PublicKeyBinding, ...]
    grants: tuple[AuthorizationGrant, ...]
    created_unix_ms: int
    updated_unix_ms: int
    recovery: RecoveryPolicy | None = None


@dataclass(frozen=True)
class CreatePrincipalRequest:
    command: PrincipalCommand
    principal_ura: str


@dataclass(frozen=True)
class BindPrincipalKeyRequest:
    command: PrincipalCommand
    principal_ura: str
    public_key: bytes
    key_id: str = ""
    expires_unix_ms: int | None = None


@dataclass(frozen=True)
class RotatePrincipalKeyRequest:
    command: PrincipalCommand
    principal_ura: str
    binding_id: str
    replacement: BindPrincipalKeyRequest


@dataclass(frozen=True)
class RevokePrincipalKeyRequest:
    command: PrincipalCommand
    principal_ura: str
    binding_id: str


@dataclass(frozen=True)
class ConfigureRecoveryRequest:
    command: PrincipalCommand
    principal_ura: str
    policy_ref: str


@dataclass(frozen=True)
class RecoverPrincipalRequest:
    command: PrincipalCommand
    principal_ura: str
    replacement_key: BindPrincipalKeyRequest


@dataclass(frozen=True)
class ChangePrincipalStateRequest:
    command: PrincipalCommand
    principal_ura: str


@dataclass(frozen=True)
class IssueGrantRequest:
    command: PrincipalCommand
    principal_ura: str
    actions: tuple[str, ...]
    expires_unix_ms: int | None = None


@dataclass(frozen=True)
class RevokeGrantRequest:
    command: PrincipalCommand
    principal_ura: str
    grant_id: str


class PrincipalLifecycle(Protocol):
    def create(self, request: CreatePrincipalRequest) -> PrincipalSnapshot: ...
    def bind_first_key(self, request: BindPrincipalKeyRequest) -> PrincipalSnapshot: ...
    def add_key(self, request: BindPrincipalKeyRequest) -> PrincipalSnapshot: ...
    def rotate_key(self, request: RotatePrincipalKeyRequest) -> PrincipalSnapshot: ...
    def revoke_key(self, request: RevokePrincipalKeyRequest) -> PrincipalSnapshot: ...
    def configure_recovery(self, request: ConfigureRecoveryRequest) -> PrincipalSnapshot: ...
    def recover(self, request: RecoverPrincipalRequest) -> PrincipalSnapshot: ...
    def suspend(self, request: ChangePrincipalStateRequest) -> PrincipalSnapshot: ...
    def reactivate(self, request: ChangePrincipalStateRequest) -> PrincipalSnapshot: ...
    def delete(self, request: ChangePrincipalStateRequest) -> PrincipalSnapshot: ...
    def issue_grant(self, request: IssueGrantRequest) -> PrincipalSnapshot: ...
    def revoke_grant(self, request: RevokeGrantRequest) -> PrincipalSnapshot: ...
    def get(self, principal_ura: str) -> PrincipalSnapshot: ...


def grant_actions(actions: Sequence[str]) -> tuple[str, ...]:
    """Return an immutable action projection without applying product policy."""
    return tuple(actions)
