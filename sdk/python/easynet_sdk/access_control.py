"""Product-neutral access-control facade over daemon authority bindings."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import StrEnum
from typing import Mapping, Protocol, Sequence

from .axon_addressing import parse_ura
from .errors import ErrorCode, RetryHint, SDKError
from .runtime_ability import RuntimeCallContext

__all__ = [
    "AccessControlAuthorityProof",
    "AccessControlCheckRequest",
    "AccessControlCheckResult",
    "AccessControlClient",
    "AccessControlEffect",
    "AccessControlGrant",
    "AccessControlGrantRequest",
    "AccessControlGrantResult",
    "AccessControlGrantState",
    "AccessControlListRequest",
    "AccessControlListResult",
    "AccessControlPolicyDecision",
    "AccessControlPrincipalKind",
    "AccessControlProvider",
    "AccessControlRevokeRequest",
    "AccessControlSignatureDecision",
    "RuntimeAccessControlProvider",
]

_ABILITY_GRANT = "authority.binding.grant"
_ABILITY_REVOKE = "authority.binding.revoke"
_ABILITY_LIST = "authority.binding.list"
_ABILITY_CHECK = "authority.binding.check"


class AccessControlGrantState(StrEnum):
    ACTIVE = "active"
    EXPIRED = "expired"
    REVOKED = "revoked"


class AccessControlEffect(StrEnum):
    ALLOW = "allow"
    DENY = "deny"


class AccessControlPrincipalKind(StrEnum):
    USER = "user"
    TOKEN = "token"
    AGENT = "agent"


@dataclass(frozen=True)
class AccessControlGrant:
    grant_id: str
    principal_kind: AccessControlPrincipalKind
    actions: tuple[str, ...]
    created_by: str
    owner_ura: str = ""
    principal_ura: str = ""
    token_id: str = ""
    callee_ura: str = ""
    ability_ura_pattern: str = ""
    subject_ura_pattern: str = ""
    effect: AccessControlEffect = AccessControlEffect.ALLOW
    state: AccessControlGrantState = AccessControlGrantState.ACTIVE
    created_at: str = ""
    expires_at: str = ""
    revoked_at: str = ""
    revoked_by: str = ""
    revocation_reason: str = ""
    constraints: Mapping[str, object] = field(default_factory=dict)
    authority_proof_id: str = ""
    source_request_id: str = ""
    invocation_template: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class AccessControlPolicyDecision:
    decision: str
    reason: str = ""
    owner_ura: str = ""
    owner_source: str = ""
    principal_kind: AccessControlPrincipalKind | None = None
    principal_ura: str = ""
    token_id: str = ""
    callee_ura: str = ""
    ability_ura: str = ""
    subject_ura: str = ""
    action: str = ""
    grant_id: str = ""
    rejector_ura: str = ""
    authority_proof_id: str = ""
    audit_warnings: tuple[str, ...] = ()


@dataclass(frozen=True)
class AccessControlSignatureDecision:
    decision: str
    reason: str = ""
    caller_ura: str = ""
    callee_ura: str = ""
    ability_ura: str = ""
    subject_ura: str = ""
    canonical_hash: str = ""
    signature_key_id: str = ""
    rejector_ura: str = ""


@dataclass(frozen=True)
class AccessControlAuthorityProof:
    proof_id: str
    grant_id: str = ""
    owner_ura: str = ""
    principal_kind: AccessControlPrincipalKind | None = None
    principal_ura: str = ""
    ability_ura: str = ""
    subject_ura: str = ""
    action: str = ""
    issuer_ura: str = ""
    issued_at: str = ""
    expires_at: str = ""
    canonical_invocation_hash: str = ""
    signature: str = ""
    verification_key_id: str = ""


@dataclass(frozen=True)
class AccessControlGrantRequest:
    call: RuntimeCallContext
    grant: AccessControlGrant
    owner_ura: str = ""
    principal_ura: str = ""
    actor_ura: str = ""


@dataclass(frozen=True)
class AccessControlRevokeRequest:
    call: RuntimeCallContext
    owner_ura: str
    grant_id: str
    actor_ura: str = ""
    reason: str = ""


@dataclass(frozen=True)
class AccessControlListRequest:
    call: RuntimeCallContext
    owner_ura: str
    principal_kind: AccessControlPrincipalKind | None = None
    principal_ura: str = ""
    token_id: str = ""
    callee_ura: str = ""
    ability_ura_pattern: str = ""
    subject_ura_pattern: str = ""
    action: str = ""
    effect: AccessControlEffect | None = None
    state: AccessControlGrantState | None = None
    limit: int = 0


@dataclass(frozen=True)
class AccessControlCheckRequest:
    call: RuntimeCallContext
    owner_ura: str
    principal_kind: AccessControlPrincipalKind
    principal_ura: str
    callee_ura: str
    subject_ura: str
    ability_ura: str
    action: str
    owner_source: str = ""
    caller_ura: str = ""
    token_id: str = ""
    token_class: str = ""
    safe_read: bool = False
    interactive_context_available: bool = False
    canonical_hash: str = ""
    signature_key_id: str = ""
    authority_proof_id: str = ""
    rejector_ura: str = ""


@dataclass(frozen=True)
class AccessControlGrantResult:
    grant: AccessControlGrant
    audit_record_id: str = ""


@dataclass(frozen=True)
class AccessControlListResult:
    grants: tuple[AccessControlGrant, ...]


@dataclass(frozen=True)
class AccessControlCheckResult:
    policy_decision: AccessControlPolicyDecision


class AccessControlProvider(Protocol):
    def grant(self, request: AccessControlGrantRequest) -> AccessControlGrantResult: ...
    def revoke(self, request: AccessControlRevokeRequest) -> AccessControlGrant: ...
    def list(self, request: AccessControlListRequest) -> AccessControlListResult: ...
    def check(self, request: AccessControlCheckRequest) -> AccessControlCheckResult: ...


class AccessControlClient:
    def __init__(self, provider: AccessControlProvider) -> None:
        if provider is None:
            raise _invalid("AccessControl provider is required")
        self._provider = provider

    def grant(self, request: AccessControlGrantRequest) -> AccessControlGrantResult:
        return self._provider.grant(request)

    def revoke(self, request: AccessControlRevokeRequest) -> AccessControlGrant:
        return self._provider.revoke(request)

    def list(self, request: AccessControlListRequest) -> AccessControlListResult:
        return self._provider.list(request)

    def check(self, request: AccessControlCheckRequest) -> AccessControlCheckResult:
        return self._provider.check(request)


class _RuntimeAbilityInvoker(Protocol):
    def invoke(
        self, call: RuntimeCallContext, ability_name: str, arguments: object
    ) -> dict[str, object]: ...


class RuntimeAccessControlProvider:
    def __init__(self, ability: _RuntimeAbilityInvoker) -> None:
        if ability is None:
            raise _invalid("runtime ability client is required")
        self._ability = ability

    def grant(self, request: AccessControlGrantRequest) -> AccessControlGrantResult:
        normalized, args = _grant_args(request)
        output = self._ability.invoke(normalized.call, _ABILITY_GRANT, args)
        grant = _grant_from_mapping(_mapping(output.get("grant"), "grant"))
        return AccessControlGrantResult(
            grant=grant,
            audit_record_id=_text(output.get("audit_record_id")),
        )

    def revoke(self, request: AccessControlRevokeRequest) -> AccessControlGrant:
        normalized, args = _revoke_args(request)
        output = self._ability.invoke(normalized.call, _ABILITY_REVOKE, args)
        return _grant_from_mapping(_mapping(output.get("grant"), "grant"))

    def list(self, request: AccessControlListRequest) -> AccessControlListResult:
        normalized, args = _list_args(request)
        output = self._ability.invoke(normalized.call, _ABILITY_LIST, args)
        raw = output.get("grants")
        if not isinstance(raw, Sequence) or isinstance(raw, (str, bytes, bytearray)):
            raise _invalid("access-control grants projection is required")
        grants = tuple(_grant_from_mapping(_mapping(item, "grant")) for item in raw)
        return AccessControlListResult(grants=grants)

    def check(self, request: AccessControlCheckRequest) -> AccessControlCheckResult:
        normalized, args = _check_args(request)
        output = self._ability.invoke(normalized.call, _ABILITY_CHECK, args)
        decision = _policy_decision_from_mapping(
            _mapping(output.get("policy_decision"), "policy_decision")
        )
        return AccessControlCheckResult(policy_decision=decision)


def _grant_args(
    request: AccessControlGrantRequest,
) -> tuple[AccessControlGrantRequest, dict[str, object]]:
    grant = _normalize_grant(request.grant, request.owner_ura, request.principal_ura)
    owner_user_id = _user_id_from_user_ura(grant.owner_ura, "owner_ura")
    principal_id = _principal_id(grant.principal_kind, grant.principal_ura, "")
    wire_grant = _grant_wire(grant, owner_user_id, principal_id)
    args: dict[str, object] = {
        "grant": wire_grant,
        "owner_ura": grant.owner_ura,
    }
    if grant.principal_ura:
        args["principal_ura"] = grant.principal_ura
    if request.actor_ura.strip():
        args["actor_ura"] = request.actor_ura.strip()
    return (
        AccessControlGrantRequest(
            call=request.call,
            grant=grant,
            owner_ura=grant.owner_ura,
            principal_ura=grant.principal_ura,
            actor_ura=request.actor_ura.strip(),
        ),
        args,
    )


def _revoke_args(
    request: AccessControlRevokeRequest,
) -> tuple[AccessControlRevokeRequest, dict[str, object]]:
    owner_ura = _required_text(request.owner_ura, "owner_ura")
    owner_user_id = _user_id_from_user_ura(owner_ura, "owner_ura")
    grant_id = _required_text(request.grant_id, "grant_id")
    args: dict[str, object] = {
        "owner_ura": owner_ura,
        "owner_user_id": owner_user_id,
        "grant_id": grant_id,
    }
    _optional(args, "actor_ura", request.actor_ura)
    _optional(args, "reason", request.reason)
    return request, args


def _list_args(
    request: AccessControlListRequest,
) -> tuple[AccessControlListRequest, dict[str, object]]:
    owner_ura = _required_text(request.owner_ura, "owner_ura")
    owner_user_id = _user_id_from_user_ura(owner_ura, "owner_ura")
    principal_id = _principal_id(request.principal_kind, request.principal_ura, "")
    args: dict[str, object] = {"owner_ura": owner_ura, "owner_user_id": owner_user_id}
    if request.principal_kind is not None:
        args["principal_kind"] = request.principal_kind.value
    if principal_id:
        args["principal_id"] = principal_id
    _optional(args, "principal_ura", request.principal_ura)
    _optional(args, "token_id", request.token_id)
    _optional(args, "callee_ura", request.callee_ura)
    _optional(args, "ability_ura_pattern", request.ability_ura_pattern)
    _optional(args, "subject_ura_pattern", request.subject_ura_pattern)
    _optional(args, "action", request.action)
    if request.effect is not None:
        args["effect"] = request.effect.value
    if request.state is not None:
        args["state"] = request.state.value
    if request.limit:
        args["limit"] = request.limit
    return request, args


def _check_args(
    request: AccessControlCheckRequest,
) -> tuple[AccessControlCheckRequest, dict[str, object]]:
    owner_ura = _required_text(request.owner_ura, "owner_ura")
    owner_user_id = _user_id_from_user_ura(owner_ura, "owner_ura")
    principal_id = _principal_id(request.principal_kind, request.principal_ura, "")
    if not principal_id:
        raise _invalid("principal_ura is required")
    for field, value in {
        "callee_ura": request.callee_ura,
        "subject_ura": request.subject_ura,
        "ability_ura": request.ability_ura,
        "action": request.action,
    }.items():
        _required_text(value, field)
    args: dict[str, object] = {
        "owner_ura": owner_ura,
        "owner_user_id": owner_user_id,
        "principal_kind": request.principal_kind.value,
        "principal_id": principal_id,
        "principal_ura": request.principal_ura.strip(),
        "callee_ura": request.callee_ura.strip(),
        "subject_ura": request.subject_ura.strip(),
        "ability_ura": request.ability_ura.strip(),
        "action": request.action.strip(),
        "safe_read": request.safe_read,
        "interactive_context_available": request.interactive_context_available,
    }
    _optional(args, "owner_source", request.owner_source)
    _optional(args, "caller_ura", request.caller_ura)
    _optional(args, "token_id", request.token_id)
    _optional(args, "token_class", request.token_class)
    _optional(args, "canonical_hash", request.canonical_hash)
    _optional(args, "signature_key_id", request.signature_key_id)
    _optional(args, "authority_proof_id", request.authority_proof_id)
    _optional(args, "rejector_ura", request.rejector_ura)
    return request, args


def _normalize_grant(
    grant: AccessControlGrant, owner_ura: str, principal_ura: str
) -> AccessControlGrant:
    effective_owner = owner_ura.strip() or grant.owner_ura.strip()
    _user_id_from_user_ura(effective_owner, "owner_ura")
    effective_principal = principal_ura.strip() or grant.principal_ura.strip()
    _principal_id(grant.principal_kind, effective_principal, "")
    if not grant.grant_id.strip():
        raise _invalid("grant_id is required")
    if not grant.actions:
        raise _invalid("grant actions are required")
    return AccessControlGrant(
        grant_id=grant.grant_id.strip(),
        owner_ura=effective_owner,
        principal_kind=grant.principal_kind,
        principal_ura=effective_principal,
        token_id=grant.token_id.strip(),
        callee_ura=grant.callee_ura.strip(),
        ability_ura_pattern=grant.ability_ura_pattern.strip(),
        subject_ura_pattern=grant.subject_ura_pattern.strip(),
        actions=tuple(grant.actions),
        effect=grant.effect,
        state=grant.state,
        created_by=grant.created_by.strip(),
        created_at=grant.created_at.strip(),
        expires_at=grant.expires_at.strip(),
        revoked_at=grant.revoked_at.strip(),
        revoked_by=grant.revoked_by.strip(),
        revocation_reason=grant.revocation_reason.strip(),
        constraints=dict(grant.constraints),
        authority_proof_id=grant.authority_proof_id.strip(),
        source_request_id=grant.source_request_id.strip(),
        invocation_template=dict(grant.invocation_template),
    )


def _grant_wire(
    grant: AccessControlGrant, owner_user_id: str, principal_id: str
) -> dict[str, object]:
    wire: dict[str, object] = {
        "grant_id": grant.grant_id,
        "owner_user_id": owner_user_id,
        "owner_ura": grant.owner_ura,
        "principal_kind": grant.principal_kind.value,
        "principal_id": principal_id,
        "principal_ura": grant.principal_ura,
        "actions": list(grant.actions),
        "effect": grant.effect.value,
        "state": grant.state.value,
        "created_by": grant.created_by,
        "constraints": dict(grant.constraints),
    }
    for key, value in {
        "token_id": grant.token_id,
        "callee_ura": grant.callee_ura,
        "ability_ura_pattern": grant.ability_ura_pattern,
        "subject_ura_pattern": grant.subject_ura_pattern,
        "created_at": grant.created_at,
        "expires_at": grant.expires_at,
        "revoked_at": grant.revoked_at,
        "revoked_by": grant.revoked_by,
        "revocation_reason": grant.revocation_reason,
        "authority_proof_id": grant.authority_proof_id,
        "source_request_id": grant.source_request_id,
    }.items():
        _optional(wire, key, value)
    if grant.invocation_template:
        wire["invocation_template"] = dict(grant.invocation_template)
    return wire


def _user_id_from_user_ura(value: str, field_name: str) -> str:
    projection = parse_ura(_required_text(value, field_name))
    if projection.kind != "user":
        raise _invalid(f"{field_name} must be a canonical User URA")
    user_id = projection.components.get("user_id")
    if not isinstance(user_id, str) or not user_id.strip():
        raise _invalid(f"{field_name} must include a user id")
    return user_id.strip()


def _principal_id(
    kind: AccessControlPrincipalKind | None, principal_ura: str, principal_id: str
) -> str:
    principal_ura = principal_ura.strip()
    principal_id = principal_id.strip()
    if kind is None:
        kind = AccessControlPrincipalKind.USER
    if not principal_ura:
        return principal_id
    projection = parse_ura(principal_ura)
    if kind == AccessControlPrincipalKind.USER:
        if projection.kind != "user":
            raise _invalid("principal_ura for user principal must be a User URA")
        canonical = projection.components.get("user_id")
        if not isinstance(canonical, str) or not canonical.strip():
            raise _invalid("principal_ura must include a user id")
        result = canonical.strip()
    elif kind == AccessControlPrincipalKind.TOKEN:
        if not principal_id:
            raise _invalid("principal_id is required for token principals")
        result = principal_id
    else:
        result = principal_ura
    if principal_id and principal_id != result:
        raise _invalid("principal_id must match principal_ura")
    return result


def _grant_from_mapping(raw: Mapping[str, object]) -> AccessControlGrant:
    grant_id = _required_text(_text(raw.get("grant_id")), "grant_id")
    return AccessControlGrant(
        grant_id=grant_id,
        owner_ura=_text(raw.get("owner_ura")),
        principal_kind=_principal_kind(raw.get("principal_kind")),
        principal_ura=_text(raw.get("principal_ura")),
        token_id=_text(raw.get("token_id")),
        callee_ura=_text(raw.get("callee_ura")),
        ability_ura_pattern=_text(raw.get("ability_ura_pattern")),
        subject_ura_pattern=_text(raw.get("subject_ura_pattern")),
        actions=_string_tuple(raw.get("actions")),
        effect=_effect(raw.get("effect")),
        state=_grant_state(raw.get("state")),
        created_by=_text(raw.get("created_by")),
        created_at=_text(raw.get("created_at")),
        expires_at=_text(raw.get("expires_at")),
        revoked_at=_text(raw.get("revoked_at")),
        revoked_by=_text(raw.get("revoked_by")),
        revocation_reason=_text(raw.get("revocation_reason")),
        constraints=_mapping_or_empty(raw.get("constraints")),
        authority_proof_id=_text(raw.get("authority_proof_id")),
        source_request_id=_text(raw.get("source_request_id")),
        invocation_template=_mapping_or_empty(raw.get("invocation_template")),
    )


def _policy_decision_from_mapping(raw: Mapping[str, object]) -> AccessControlPolicyDecision:
    decision = _required_text(_text(raw.get("decision")), "decision")
    return AccessControlPolicyDecision(
        decision=decision,
        reason=_text(raw.get("reason")),
        owner_ura=_text(raw.get("owner_ura")),
        owner_source=_text(raw.get("owner_source")),
        principal_kind=_optional_principal_kind(raw.get("principal_kind")),
        principal_ura=_text(raw.get("principal_ura")),
        token_id=_text(raw.get("token_id")),
        callee_ura=_text(raw.get("callee_ura")),
        ability_ura=_text(raw.get("ability_ura")),
        subject_ura=_text(raw.get("subject_ura")),
        action=_text(raw.get("action")),
        grant_id=_text(raw.get("grant_id")),
        rejector_ura=_text(raw.get("rejector_ura")),
        authority_proof_id=_text(raw.get("authority_proof_id")),
        audit_warnings=_string_tuple(raw.get("audit_warnings")),
    )


def _principal_kind(value: object) -> AccessControlPrincipalKind:
    text = _text(value) or AccessControlPrincipalKind.USER.value
    try:
        return AccessControlPrincipalKind(text)
    except ValueError as exc:
        raise _invalid(f"unknown principal kind: {text}", exc) from exc


def _optional_principal_kind(value: object) -> AccessControlPrincipalKind | None:
    text = _text(value)
    return _principal_kind(text) if text else None


def _effect(value: object) -> AccessControlEffect:
    text = _text(value) or AccessControlEffect.ALLOW.value
    try:
        return AccessControlEffect(text)
    except ValueError as exc:
        raise _invalid(f"unknown access-control effect: {text}", exc) from exc


def _grant_state(value: object) -> AccessControlGrantState:
    text = _text(value) or AccessControlGrantState.ACTIVE.value
    try:
        return AccessControlGrantState(text)
    except ValueError as exc:
        raise _invalid(f"unknown access-control grant state: {text}", exc) from exc


def _required_text(value: object, field_name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _invalid(f"{field_name} is required")
    return value.strip()


def _text(value: object) -> str:
    return value if isinstance(value, str) else ""


def _string_tuple(value: object) -> tuple[str, ...]:
    if not isinstance(value, Sequence) or isinstance(value, (str, bytes, bytearray)):
        return ()
    return tuple(item for item in value if isinstance(item, str))


def _mapping(value: object, field_name: str) -> Mapping[str, object]:
    if isinstance(value, Mapping):
        return value
    raise _invalid(f"{field_name} projection is required")


def _mapping_or_empty(value: object) -> Mapping[str, object]:
    return dict(value) if isinstance(value, Mapping) else {}


def _optional(target: dict[str, object], key: str, value: object) -> None:
    if isinstance(value, str) and value.strip():
        target[key] = value.strip()


def _invalid(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="access_control",
        retry=RetryHint.NEVER,
        message=message,
        details={"profile": "access_control"},
        cause=cause,
    )
