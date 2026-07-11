"""Product-neutral principal lifecycle contracts.

The module exposes immutable public projections and a provider protocol. It
does not own account login, private-key custody, or a product-specific user
directory.
"""

from __future__ import annotations

import base64
from dataclasses import dataclass
from enum import Enum
from typing import Mapping, Protocol, Sequence

from .errors import ErrorCode, RetryHint, SDKError
from .runtime_ability import RuntimeCallContext

__all__ = [
    "AuthorizationGrant",
    "BindPrincipalKeyRequest",
    "ChangePrincipalStateRequest",
    "ConfigureRecoveryRequest",
    "CreatePrincipalRequest",
    "EnrollmentCapability",
    "IssueEnrollmentRequest",
    "IssueGrantRequest",
    "PrincipalCommand",
    "PrincipalClient",
    "PrincipalLifecycle",
    "PrincipalProvider",
    "PrincipalProofKind",
    "PrincipalProofRef",
    "PrincipalSnapshot",
    "PrincipalState",
    "PublicKeyBinding",
    "PublicKeyBindingState",
    "RecoverPrincipalRequest",
    "RecoveryPolicy",
    "RevokeEnrollmentRequest",
    "RevokeGrantRequest",
    "RevokePrincipalKeyRequest",
    "RuntimePrincipalProvider",
    "RotatePrincipalKeyRequest",
    "grant_actions",
]

_PROFILE = "principal_lifecycle"

_ABILITY_CREATE = "principal.lifecycle.create"
_ABILITY_BIND_FIRST_KEY = "principal.lifecycle.bind_first_key"
_ABILITY_ADD_KEY = "principal.lifecycle.add_key"
_ABILITY_ROTATE_KEY = "principal.lifecycle.rotate_key"
_ABILITY_REVOKE_KEY = "principal.lifecycle.revoke_key"
_ABILITY_CONFIGURE_RECOVERY = "principal.lifecycle.configure_recovery"
_ABILITY_RECOVER = "principal.lifecycle.recover"
_ABILITY_SUSPEND = "principal.lifecycle.suspend"
_ABILITY_REACTIVATE = "principal.lifecycle.reactivate"
_ABILITY_DELETE = "principal.lifecycle.delete"
_ABILITY_ISSUE_ENROLLMENT = "principal.lifecycle.issue_enrollment"
_ABILITY_REVOKE_ENROLLMENT = "principal.lifecycle.revoke_enrollment"
_ABILITY_ISSUE_GRANT = "principal.lifecycle.issue_grant"
_ABILITY_REVOKE_GRANT = "principal.lifecycle.revoke_grant"
_ABILITY_GET = "principal.lifecycle.get"

_PRIVATE_PROJECTION_FIELD_TOKENS = (
    "seed",
    "private",
    "secret",
    "vault",
    "passphrase",
    "master_key",
    "ciphertext",
    "keyring",
    "storage_path",
)


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
class EnrollmentCapability:
    enrollment_id: str
    issuer_ura: str
    subject_principal_ura: str
    created_unix_ms: int
    expires_unix_ms: int | None = None
    revoked_unix_ms: int | None = None
    consumed_by_principal_ura: str = ""
    consumed_unix_ms: int | None = None


@dataclass(frozen=True)
class PrincipalSnapshot:
    principal_ura: str
    state: PrincipalState
    version: int
    bindings: tuple[PublicKeyBinding, ...]
    grants: tuple[AuthorizationGrant, ...]
    created_unix_ms: int
    updated_unix_ms: int
    enrollment_proof: PrincipalProofRef | None = None
    recovery: RecoveryPolicy | None = None
    enrollments: tuple[EnrollmentCapability, ...] = ()


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
class IssueEnrollmentRequest:
    command: PrincipalCommand
    principal_ura: str
    subject_principal_ura: str
    expires_unix_ms: int | None = None


@dataclass(frozen=True)
class RevokeEnrollmentRequest:
    command: PrincipalCommand
    principal_ura: str
    enrollment_id: str


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


class PrincipalProvider(Protocol):
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
    def issue_enrollment(self, request: IssueEnrollmentRequest) -> PrincipalSnapshot: ...
    def revoke_enrollment(self, request: RevokeEnrollmentRequest) -> PrincipalSnapshot: ...
    def issue_grant(self, request: IssueGrantRequest) -> PrincipalSnapshot: ...
    def revoke_grant(self, request: RevokeGrantRequest) -> PrincipalSnapshot: ...
    def get(self, principal_ura: str) -> PrincipalSnapshot: ...


PrincipalLifecycle = PrincipalProvider


class PrincipalClient:
    def __init__(self, provider: PrincipalProvider) -> None:
        if provider is None:
            raise _invalid("Principal provider is required")
        self._provider = provider

    def create(self, request: CreatePrincipalRequest) -> PrincipalSnapshot:
        return self._provider.create(request)

    def bind_first_key(self, request: BindPrincipalKeyRequest) -> PrincipalSnapshot:
        return self._provider.bind_first_key(request)

    def add_key(self, request: BindPrincipalKeyRequest) -> PrincipalSnapshot:
        return self._provider.add_key(request)

    def rotate_key(self, request: RotatePrincipalKeyRequest) -> PrincipalSnapshot:
        return self._provider.rotate_key(request)

    def revoke_key(self, request: RevokePrincipalKeyRequest) -> PrincipalSnapshot:
        return self._provider.revoke_key(request)

    def configure_recovery(self, request: ConfigureRecoveryRequest) -> PrincipalSnapshot:
        return self._provider.configure_recovery(request)

    def recover(self, request: RecoverPrincipalRequest) -> PrincipalSnapshot:
        return self._provider.recover(request)

    def suspend(self, request: ChangePrincipalStateRequest) -> PrincipalSnapshot:
        return self._provider.suspend(request)

    def reactivate(self, request: ChangePrincipalStateRequest) -> PrincipalSnapshot:
        return self._provider.reactivate(request)

    def delete(self, request: ChangePrincipalStateRequest) -> PrincipalSnapshot:
        return self._provider.delete(request)

    def issue_enrollment(self, request: IssueEnrollmentRequest) -> PrincipalSnapshot:
        return self._provider.issue_enrollment(request)

    def revoke_enrollment(self, request: RevokeEnrollmentRequest) -> PrincipalSnapshot:
        return self._provider.revoke_enrollment(request)

    def issue_grant(self, request: IssueGrantRequest) -> PrincipalSnapshot:
        return self._provider.issue_grant(request)

    def revoke_grant(self, request: RevokeGrantRequest) -> PrincipalSnapshot:
        return self._provider.revoke_grant(request)

    def get(self, principal_ura: str) -> PrincipalSnapshot:
        return self._provider.get(principal_ura)


class _RuntimeAbilityInvoker(Protocol):
    def invoke(
        self, call: RuntimeCallContext, ability_name: str, arguments: object
    ) -> dict[str, object]: ...


class RuntimePrincipalProvider:
    def __init__(self, ability: _RuntimeAbilityInvoker, call: RuntimeCallContext) -> None:
        if ability is None:
            raise _invalid("runtime ability client is required")
        if not (call.caller_ura.strip() and call.callee_ura.strip() and call.subject_ura.strip()):
            raise _invalid("runtime call context requires caller_ura, callee_ura and subject_ura")
        self._ability = ability
        self._call = call

    def create(self, request: CreatePrincipalRequest) -> PrincipalSnapshot:
        return self._invoke(_ABILITY_CREATE, {"request": _create_wire(request)})

    def bind_first_key(self, request: BindPrincipalKeyRequest) -> PrincipalSnapshot:
        return self._invoke(_ABILITY_BIND_FIRST_KEY, {"request": _bind_key_wire(request)})

    def add_key(self, request: BindPrincipalKeyRequest) -> PrincipalSnapshot:
        return self._invoke(_ABILITY_ADD_KEY, {"request": _bind_key_wire(request)})

    def rotate_key(self, request: RotatePrincipalKeyRequest) -> PrincipalSnapshot:
        return self._invoke(_ABILITY_ROTATE_KEY, {"request": _rotate_key_wire(request)})

    def revoke_key(self, request: RevokePrincipalKeyRequest) -> PrincipalSnapshot:
        return self._invoke(_ABILITY_REVOKE_KEY, {"request": _revoke_key_wire(request)})

    def configure_recovery(self, request: ConfigureRecoveryRequest) -> PrincipalSnapshot:
        return self._invoke(
            _ABILITY_CONFIGURE_RECOVERY, {"request": _configure_recovery_wire(request)}
        )

    def recover(self, request: RecoverPrincipalRequest) -> PrincipalSnapshot:
        return self._invoke(_ABILITY_RECOVER, {"request": _recover_wire(request)})

    def suspend(self, request: ChangePrincipalStateRequest) -> PrincipalSnapshot:
        return self._invoke(_ABILITY_SUSPEND, {"request": _change_state_wire(request)})

    def reactivate(self, request: ChangePrincipalStateRequest) -> PrincipalSnapshot:
        return self._invoke(_ABILITY_REACTIVATE, {"request": _change_state_wire(request)})

    def delete(self, request: ChangePrincipalStateRequest) -> PrincipalSnapshot:
        return self._invoke(_ABILITY_DELETE, {"request": _change_state_wire(request)})

    def issue_enrollment(self, request: IssueEnrollmentRequest) -> PrincipalSnapshot:
        return self._invoke(
            _ABILITY_ISSUE_ENROLLMENT, {"request": _issue_enrollment_wire(request)}
        )

    def revoke_enrollment(self, request: RevokeEnrollmentRequest) -> PrincipalSnapshot:
        return self._invoke(
            _ABILITY_REVOKE_ENROLLMENT, {"request": _revoke_enrollment_wire(request)}
        )

    def issue_grant(self, request: IssueGrantRequest) -> PrincipalSnapshot:
        return self._invoke(_ABILITY_ISSUE_GRANT, {"request": _issue_grant_wire(request)})

    def revoke_grant(self, request: RevokeGrantRequest) -> PrincipalSnapshot:
        return self._invoke(_ABILITY_REVOKE_GRANT, {"request": _revoke_grant_wire(request)})

    def get(self, principal_ura: str) -> PrincipalSnapshot:
        principal_ura = principal_ura.strip()
        if not principal_ura:
            raise _invalid("principal_ura is required")
        return self._invoke(_ABILITY_GET, {"principal_ura": principal_ura})

    def _invoke(self, ability: str, arguments: dict[str, object]) -> PrincipalSnapshot:
        metadata = dict(self._call.metadata)
        metadata["profile"] = _PROFILE
        metadata["system_ability"] = ability
        call = RuntimeCallContext(
            caller_ura=self._call.caller_ura,
            callee_ura=self._call.callee_ura,
            subject_ura=self._call.subject_ura,
            nonce_base64=self._call.nonce_base64,
            causal_context=self._call.causal_context,
            descriptor_version=self._call.descriptor_version,
            metadata=metadata,
        )
        output = self._ability.invoke(call, ability, arguments)
        return _snapshot_from_mapping(_mapping(output.get("principal"), "principal"))


def grant_actions(actions: Sequence[str]) -> tuple[str, ...]:
    """Return an immutable action projection without applying product policy."""
    return tuple(actions)


def _create_wire(request: CreatePrincipalRequest) -> dict[str, object]:
    return {
        "command": _command_wire(request.command),
        "principal_ura": request.principal_ura.strip(),
    }


def _bind_key_wire(request: BindPrincipalKeyRequest) -> dict[str, object]:
    wire: dict[str, object] = {
        "command": _command_wire(request.command),
        "principal_ura": request.principal_ura.strip(),
        "public_key_b64": base64.b64encode(request.public_key).decode("ascii"),
    }
    _optional(wire, "key_id", request.key_id)
    if request.expires_unix_ms is not None:
        wire["expires_unix_ms"] = request.expires_unix_ms
    return wire


def _rotate_key_wire(request: RotatePrincipalKeyRequest) -> dict[str, object]:
    return {
        "command": _command_wire(request.command),
        "principal_ura": request.principal_ura.strip(),
        "binding_id": request.binding_id.strip(),
        "replacement": _bind_key_wire(request.replacement),
    }


def _revoke_key_wire(request: RevokePrincipalKeyRequest) -> dict[str, object]:
    return {
        "command": _command_wire(request.command),
        "principal_ura": request.principal_ura.strip(),
        "binding_id": request.binding_id.strip(),
    }


def _configure_recovery_wire(request: ConfigureRecoveryRequest) -> dict[str, object]:
    return {
        "command": _command_wire(request.command),
        "principal_ura": request.principal_ura.strip(),
        "policy_ref": request.policy_ref.strip(),
    }


def _recover_wire(request: RecoverPrincipalRequest) -> dict[str, object]:
    return {
        "command": _command_wire(request.command),
        "principal_ura": request.principal_ura.strip(),
        "replacement_key": _bind_key_wire(request.replacement_key),
    }


def _change_state_wire(request: ChangePrincipalStateRequest) -> dict[str, object]:
    return {
        "command": _command_wire(request.command),
        "principal_ura": request.principal_ura.strip(),
    }


def _issue_enrollment_wire(request: IssueEnrollmentRequest) -> dict[str, object]:
    wire: dict[str, object] = {
        "command": _command_wire(request.command),
        "principal_ura": request.principal_ura.strip(),
        "subject_principal_ura": request.subject_principal_ura.strip(),
    }
    if request.expires_unix_ms is not None:
        wire["expires_unix_ms"] = request.expires_unix_ms
    return wire


def _revoke_enrollment_wire(request: RevokeEnrollmentRequest) -> dict[str, object]:
    return {
        "command": _command_wire(request.command),
        "principal_ura": request.principal_ura.strip(),
        "enrollment_id": request.enrollment_id.strip(),
    }


def _issue_grant_wire(request: IssueGrantRequest) -> dict[str, object]:
    wire: dict[str, object] = {
        "command": _command_wire(request.command),
        "principal_ura": request.principal_ura.strip(),
        "actions": list(request.actions),
    }
    if request.expires_unix_ms is not None:
        wire["expires_unix_ms"] = request.expires_unix_ms
    return wire


def _revoke_grant_wire(request: RevokeGrantRequest) -> dict[str, object]:
    return {
        "command": _command_wire(request.command),
        "principal_ura": request.principal_ura.strip(),
        "grant_id": request.grant_id.strip(),
    }


def _command_wire(command: PrincipalCommand) -> dict[str, object]:
    wire: dict[str, object] = {
        "actor_ura": command.actor_ura.strip(),
        "idempotency_key": command.idempotency_key.strip(),
        "proof": {
            "kind": command.proof.kind.value,
            "reference": command.proof.reference.strip(),
        },
    }
    if command.expected_version is not None:
        wire["expected_version"] = command.expected_version
    return wire


def _snapshot_from_mapping(raw: Mapping[str, object]) -> PrincipalSnapshot:
    _reject_private_projection_fields(raw, "principal")
    principal_ura = _required_text(raw.get("principal_ura"), "principal_ura")
    return PrincipalSnapshot(
        principal_ura=principal_ura,
        state=PrincipalState(_text(raw.get("state")) or PrincipalState.PENDING.value),
        version=_int(raw.get("version")),
        bindings=tuple(_binding_from_mapping(_mapping(item, "binding")) for item in _sequence(raw.get("bindings"))),
        grants=tuple(_grant_from_mapping(_mapping(item, "grant")) for item in _sequence(raw.get("grants"))),
        created_unix_ms=_int(raw.get("created_unix_ms")),
        updated_unix_ms=_int(raw.get("updated_unix_ms")),
        enrollment_proof=(
            _proof_from_mapping(_mapping(raw.get("enrollment_proof"), "enrollment_proof"))
            if isinstance(raw.get("enrollment_proof"), Mapping)
            else None
        ),
        recovery=(
            _recovery_from_mapping(_mapping(raw.get("recovery"), "recovery"))
            if isinstance(raw.get("recovery"), Mapping)
            else None
        ),
        enrollments=tuple(
            _enrollment_from_mapping(_mapping(item, "enrollment"))
            for item in _sequence(raw.get("enrollments"))
        ),
    )


def _reject_private_projection_fields(value: object, path: str) -> None:
    if isinstance(value, Mapping):
        for field, nested in value.items():
            if not isinstance(field, str):
                raise _invalid(f"{path} projection field names must be strings")
            normalized = field.lower()
            if any(token in normalized for token in _PRIVATE_PROJECTION_FIELD_TOKENS):
                raise _invalid(
                    f"{path} projection contains forbidden private field {field}"
                )
            _reject_private_projection_fields(nested, f"{path}.{field}")
    elif isinstance(value, Sequence) and not isinstance(value, (str, bytes, bytearray)):
        for index, nested in enumerate(value):
            _reject_private_projection_fields(nested, f"{path}[{index}]")


def _proof_from_mapping(raw: Mapping[str, object]) -> PrincipalProofRef:
    return PrincipalProofRef(
        kind=PrincipalProofKind(_required_text(raw.get("kind"), "proof.kind")),
        reference=_required_text(raw.get("reference"), "proof.reference"),
    )


def _binding_from_mapping(raw: Mapping[str, object]) -> PublicKeyBinding:
    return PublicKeyBinding(
        binding_id=_text(raw.get("binding_id")),
        principal_ura=_text(raw.get("principal_ura")),
        key_id=_text(raw.get("key_id")),
        public_key=base64.b64decode(_text(raw.get("public_key_b64")) or b""),
        state=PublicKeyBindingState(_text(raw.get("state")) or PublicKeyBindingState.ACTIVE.value),
        created_unix_ms=_int(raw.get("created_unix_ms")),
        expires_unix_ms=_optional_int(raw.get("expires_unix_ms")),
        rotated_unix_ms=_optional_int(raw.get("rotated_unix_ms")),
        revoked_unix_ms=_optional_int(raw.get("revoked_unix_ms")),
        rotated_to=_text(raw.get("rotated_to")),
    )


def _recovery_from_mapping(raw: Mapping[str, object]) -> RecoveryPolicy:
    return RecoveryPolicy(
        policy_ref=_text(raw.get("policy_ref")),
        enabled=bool(raw.get("enabled")),
        updated_unix_ms=_int(raw.get("updated_unix_ms")),
    )


def _grant_from_mapping(raw: Mapping[str, object]) -> AuthorizationGrant:
    return AuthorizationGrant(
        grant_id=_text(raw.get("grant_id")),
        principal_ura=_text(raw.get("principal_ura")),
        issuer_ura=_text(raw.get("issuer_ura")),
        actions=tuple(item for item in _sequence(raw.get("actions")) if isinstance(item, str)),
        created_unix_ms=_int(raw.get("created_unix_ms")),
        expires_unix_ms=_optional_int(raw.get("expires_unix_ms")),
        revoked_unix_ms=_optional_int(raw.get("revoked_unix_ms")),
    )


def _enrollment_from_mapping(raw: Mapping[str, object]) -> EnrollmentCapability:
    return EnrollmentCapability(
        enrollment_id=_text(raw.get("enrollment_id")),
        issuer_ura=_text(raw.get("issuer_ura")),
        subject_principal_ura=_text(raw.get("subject_principal_ura")),
        created_unix_ms=_int(raw.get("created_unix_ms")),
        expires_unix_ms=_optional_int(raw.get("expires_unix_ms")),
        revoked_unix_ms=_optional_int(raw.get("revoked_unix_ms")),
        consumed_by_principal_ura=_text(raw.get("consumed_by_principal_ura")),
        consumed_unix_ms=_optional_int(raw.get("consumed_unix_ms")),
    )


def _mapping(value: object, field: str) -> Mapping[str, object]:
    if isinstance(value, Mapping):
        return value
    raise _invalid(f"{field} projection is required")


def _sequence(value: object) -> tuple[object, ...]:
    if isinstance(value, Sequence) and not isinstance(value, (str, bytes, bytearray)):
        return tuple(value)
    return ()


def _required_text(value: object, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _invalid(f"{field} is required")
    return value.strip()


def _text(value: object) -> str:
    return value if isinstance(value, str) else ""


def _int(value: object) -> int:
    return int(value) if isinstance(value, (int, float)) else 0


def _optional_int(value: object) -> int | None:
    return _int(value) if isinstance(value, (int, float)) else None


def _optional(target: dict[str, object], key: str, value: str) -> None:
    if value.strip():
        target[key] = value.strip()


def _invalid(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage=_PROFILE,
        retry=RetryHint.NEVER,
        message=message,
        details={"profile": _PROFILE},
        cause=cause,
    )
