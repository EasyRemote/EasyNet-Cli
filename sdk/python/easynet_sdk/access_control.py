"""Product-neutral access-control facade over runtime authority bindings."""

from __future__ import annotations

from dataclasses import dataclass, field, replace
from enum import StrEnum
from typing import Mapping, Protocol, Sequence

from .axon_addressing import parse_ura
from .errors import ErrorCode, RetryHint, SDKError
from .runtime_ability import RuntimeCallContext
from ._access_control_routes import (
    _ABILITY_ADMISSION_EXPLAIN,
    _ABILITY_CHECK,
    _ABILITY_GRANT,
    _ABILITY_LIST,
    _ABILITY_POLICY_REQUEST_CREATE,
    _ABILITY_POLICY_REQUEST_LIST,
    _ABILITY_POLICY_REQUEST_RESOLVE,
    _ABILITY_REVOKE,
)

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
    "AccessControlPermissionRequest",
    "AccessControlPermissionRequestCreateRequest",
    "AccessControlPermissionRequestListRequest",
    "AccessControlPermissionRequestListResult",
    "AccessControlPermissionRequestResolutionResult",
    "AccessControlPermissionRequestResolveRequest",
    "AccessControlPrincipalKind",
    "AccessControlProvider",
    "AccessControlRevokeRequest",
    "AccessControlSignatureDecision",
    "AccessControlAbilityCallTrace",
    "AccessControlAdmissionExplainRequest",
    "AccessControlAdmissionExplainResult",
    "RuntimeAccessControlProvider",
]

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
    SERVICE = "service"
    AUTOMATION = "automation"


@dataclass(frozen=True)
class AccessControlGrant:
    grant_id: str
    principal_kind: AccessControlPrincipalKind
    actions: tuple[str, ...]
    created_by: str
    owner_ura: str = ""
    principal_id: str = ""
    principal_ura: str = ""
    token_id: str = ""
    token_class: str = ""
    callee_ura: str = ""
    ability_ura_pattern: str = ""
    subject_ura_pattern: str = ""
    effect: AccessControlEffect = AccessControlEffect.ALLOW
    lifetime: str = ""
    state: AccessControlGrantState = AccessControlGrantState.ACTIVE
    created_at: str = ""
    updated_at: str = ""
    expires_at: str = ""
    review_required_after: str = ""
    last_reviewed_at: str = ""
    last_used_at: str = ""
    revoked_at: str = ""
    revoked_by: str = ""
    revocation_reason: str = ""
    reason: str = ""
    constraints: Mapping[str, object] = field(default_factory=dict)
    authority_proof_id: str = ""
    source_request_id: str = ""
    invocation_template: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class AccessControlPolicyDecision:
    decision: str
    reason: str = ""
    owner_user_id: str = ""
    owner_ura: str = ""
    owner_source: str = ""
    caller_ura: str = ""
    principal_kind: AccessControlPrincipalKind | None = None
    principal_id: str = ""
    principal_ura: str = ""
    token_id: str = ""
    callee_ura: str = ""
    ability_ura: str = ""
    subject_ura: str = ""
    action: str = ""
    grant_id: str = ""
    policy_rule_id: str = ""
    prompt_request_id: str = ""
    canonical_hash: str = ""
    signature_key_id: str = ""
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
    presented_pubkey_fingerprint: str = ""
    verifier_ura: str = ""
    rejector_ura: str = ""


@dataclass(frozen=True)
class AccessControlAuthorityProof:
    proof_id: str
    grant_id: str = ""
    permission_request_id: str = ""
    owner_ura: str = ""
    principal_kind: AccessControlPrincipalKind | None = None
    principal_id: str = ""
    principal_ura: str = ""
    token_id: str = ""
    callee_ura: str = ""
    ability_ura: str = ""
    subject_ura: str = ""
    action: str = ""
    nonce: str = ""
    canonical_hash: str = ""
    session_id: str = ""
    session_owner_ura: str = ""
    allowed_followup_abilities: tuple[str, ...] = ()
    session_expires_at: str = ""
    audience_ura: str = ""
    issuer_ura: str = ""
    issued_at: str = ""
    expires_at: str = ""
    canonical_invocation_hash: str = ""
    signature: str = ""
    verification_key_id: str = ""


@dataclass(frozen=True)
class AccessControlPermissionRequest:
    request_id: str
    owner_ura: str = ""
    caller_ura: str = ""
    principal_kind: AccessControlPrincipalKind | None = None
    principal_id: str = ""
    principal_ura: str = ""
    token_id: str = ""
    token_class: str = ""
    callee_ura: str = ""
    subject_ura: str = ""
    ability_ura: str = ""
    action: str = ""
    nonce: str = ""
    canonical_hash: str = ""
    requested_lifetimes: tuple[str, ...] = ()
    status: str = ""
    created_at: str = ""
    expires_at: str = ""
    resolver_ura: str = ""
    resolved_lifetime: str = ""
    created_grant_id: str = ""
    authority_proof_id: str = ""
    resolved_at: str = ""
    decision_reason: str = ""


@dataclass(frozen=True)
class AccessControlAbilityCallTrace:
    invocation_id: str = ""
    parent_invocation_id: str = ""
    root_invocation_id: str = ""
    caller_ura: str = ""
    callee_ura: str = ""
    subject_ura: str = ""
    ability_ura: str = ""
    action: str = ""
    route_ref: str = ""
    execution_host_ura: str = ""
    rejector_ura: str = ""
    stage: str = ""
    signature_decision: AccessControlSignatureDecision | None = None
    policy_decision: AccessControlPolicyDecision | None = None
    authority_proof_id: str = ""
    redacted: bool = False
    child_failure_class: str = ""
    redaction_reason: str = ""
    children: tuple["AccessControlAbilityCallTrace", ...] = ()


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
class AccessControlPermissionRequestCreateRequest:
    call: RuntimeCallContext
    request: AccessControlPermissionRequest
    owner_ura: str = ""
    principal_ura: str = ""
    actor_ura: str = ""


@dataclass(frozen=True)
class AccessControlPermissionRequestResolveRequest:
    call: RuntimeCallContext
    request: AccessControlPermissionRequest
    created_grant: AccessControlGrant | None = None
    authority_proof: AccessControlAuthorityProof | None = None
    owner_ura: str = ""
    principal_ura: str = ""
    actor_ura: str = ""


@dataclass(frozen=True)
class AccessControlPermissionRequestListRequest:
    call: RuntimeCallContext
    owner_ura: str
    principal_kind: AccessControlPrincipalKind | None = None
    principal_id: str = ""
    principal_ura: str = ""
    token_id: str = ""
    status: str = ""
    limit: int = 0
    cursor: str = ""


@dataclass(frozen=True)
class AccessControlAdmissionExplainRequest:
    call: RuntimeCallContext
    observer_ura: str
    invocation_id: str = ""
    trace_id: str = ""
    root_id: str = ""


@dataclass(frozen=True)
class AccessControlListRequest:
    call: RuntimeCallContext
    owner_ura: str
    principal_kind: AccessControlPrincipalKind | None = None
    principal_id: str = ""
    principal_ura: str = ""
    token_id: str = ""
    callee_ura: str = ""
    ability_ura: str = ""
    ability_ura_pattern: str = ""
    subject_ura: str = ""
    subject_ura_pattern: str = ""
    action: str = ""
    effect: AccessControlEffect | None = None
    state: AccessControlGrantState | None = None
    limit: int = 0
    cursor: str = ""


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
    idempotent_replay: bool = False
    audit_record_id: str = ""


@dataclass(frozen=True)
class AccessControlListResult:
    grants: tuple[AccessControlGrant, ...]


@dataclass(frozen=True)
class AccessControlCheckResult:
    policy_decision: AccessControlPolicyDecision


@dataclass(frozen=True)
class AccessControlPermissionRequestResolutionResult:
    request: AccessControlPermissionRequest
    created_grant: AccessControlGrant | None = None
    authority_proof: AccessControlAuthorityProof | None = None
    idempotent_replay: bool = False


@dataclass(frozen=True)
class AccessControlPermissionRequestListResult:
    requests: tuple[AccessControlPermissionRequest, ...]


@dataclass(frozen=True)
class AccessControlAdmissionExplainResult:
    observer_ura: str
    redacted: bool = False
    root_trace: AccessControlAbilityCallTrace | None = None
    signature_decision: AccessControlSignatureDecision | None = None
    policy_decision: AccessControlPolicyDecision | None = None
    authority_reason: str = ""
    route_ref: str = ""
    rejector_ura: str = ""
    redaction_reason: str = ""


class AccessControlProvider(Protocol):
    def grant(self, request: AccessControlGrantRequest) -> AccessControlGrantResult: ...
    def revoke(self, request: AccessControlRevokeRequest) -> AccessControlGrant: ...
    def list(self, request: AccessControlListRequest) -> AccessControlListResult: ...
    def check(self, request: AccessControlCheckRequest) -> AccessControlCheckResult: ...
    def create_request(
        self, request: AccessControlPermissionRequestCreateRequest
    ) -> AccessControlPermissionRequest: ...
    def resolve_request(
        self, request: AccessControlPermissionRequestResolveRequest
    ) -> AccessControlPermissionRequestResolutionResult: ...
    def list_requests(
        self, request: AccessControlPermissionRequestListRequest
    ) -> AccessControlPermissionRequestListResult: ...
    def explain(
        self, request: AccessControlAdmissionExplainRequest
    ) -> AccessControlAdmissionExplainResult: ...


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

    def create_request(
        self, request: AccessControlPermissionRequestCreateRequest
    ) -> AccessControlPermissionRequest:
        return self._provider.create_request(request)

    def resolve_request(
        self, request: AccessControlPermissionRequestResolveRequest
    ) -> AccessControlPermissionRequestResolutionResult:
        return self._provider.resolve_request(request)

    def list_requests(
        self, request: AccessControlPermissionRequestListRequest
    ) -> AccessControlPermissionRequestListResult:
        return self._provider.list_requests(request)

    def explain(
        self, request: AccessControlAdmissionExplainRequest
    ) -> AccessControlAdmissionExplainResult:
        return self._provider.explain(request)


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
            idempotent_replay=_bool(output.get("idempotent_replay")),
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

    def create_request(
        self, request: AccessControlPermissionRequestCreateRequest
    ) -> AccessControlPermissionRequest:
        normalized, args = _permission_request_create_args(request)
        output = self._ability.invoke(
            normalized.call, _ABILITY_POLICY_REQUEST_CREATE, args
        )
        return _permission_request_from_mapping(
            _mapping(output.get("request"), "request")
        )

    def resolve_request(
        self, request: AccessControlPermissionRequestResolveRequest
    ) -> AccessControlPermissionRequestResolutionResult:
        normalized, args = _permission_request_resolve_args(request)
        output = self._ability.invoke(
            normalized.call, _ABILITY_POLICY_REQUEST_RESOLVE, args
        )
        created_grant_raw = output.get("created_grant")
        authority_proof_raw = output.get("authority_proof")
        return AccessControlPermissionRequestResolutionResult(
            request=_permission_request_from_mapping(
                _mapping(output.get("request"), "request")
            ),
            created_grant=(
                _grant_from_mapping(_mapping(created_grant_raw, "created_grant"))
                if isinstance(created_grant_raw, Mapping)
                else None
            ),
            authority_proof=(
                _authority_proof_from_mapping(
                    _mapping(authority_proof_raw, "authority_proof")
                )
                if isinstance(authority_proof_raw, Mapping)
                else None
            ),
            idempotent_replay=_bool(output.get("idempotent_replay")),
        )

    def list_requests(
        self, request: AccessControlPermissionRequestListRequest
    ) -> AccessControlPermissionRequestListResult:
        normalized, args = _permission_request_list_args(request)
        output = self._ability.invoke(
            normalized.call, _ABILITY_POLICY_REQUEST_LIST, args
        )
        raw = output.get("requests")
        if not isinstance(raw, Sequence) or isinstance(raw, (str, bytes, bytearray)):
            raise _invalid("access-control permission requests projection is required")
        return AccessControlPermissionRequestListResult(
            requests=tuple(
                _permission_request_from_mapping(_mapping(item, "request"))
                for item in raw
            )
        )

    def explain(
        self, request: AccessControlAdmissionExplainRequest
    ) -> AccessControlAdmissionExplainResult:
        normalized, args = _admission_explain_args(request)
        output = self._ability.invoke(normalized.call, _ABILITY_ADMISSION_EXPLAIN, args)
        return _admission_explain_from_mapping(output)


def _grant_args(
    request: AccessControlGrantRequest,
) -> tuple[AccessControlGrantRequest, dict[str, object]]:
    grant = _normalize_grant(request.grant, request.owner_ura, request.principal_ura)
    args: dict[str, object] = {
        "grant": _grant_wire(grant),
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
    _user_id_from_user_ura(owner_ura, "owner_ura")
    grant_id = _required_text(request.grant_id, "grant_id")
    actor_ura = _required_text(request.actor_ura, "actor_ura")
    parse_ura(actor_ura)
    args: dict[str, object] = {
        "owner_ura": owner_ura,
        "grant_id": grant_id,
        "actor_ura": actor_ura,
    }
    _optional(args, "reason", request.reason)
    return request, args


def _list_args(
    request: AccessControlListRequest,
) -> tuple[AccessControlListRequest, dict[str, object]]:
    owner_ura = _required_text(request.owner_ura, "owner_ura")
    _user_id_from_user_ura(owner_ura, "owner_ura")
    _principal_id(request.principal_kind, request.principal_ura, request.principal_id)
    args: dict[str, object] = {"owner_ura": owner_ura}
    if request.principal_kind is not None:
        args["principal_kind"] = request.principal_kind.value
    _optional(args, "principal_ura", request.principal_ura)
    _optional(args, "token_id", request.token_id)
    _optional(args, "callee_ura", request.callee_ura)
    _optional(args, "ability_ura", request.ability_ura)
    _optional(args, "ability_ura_pattern", request.ability_ura_pattern)
    _optional(args, "subject_ura", request.subject_ura)
    _optional(args, "subject_ura_pattern", request.subject_ura_pattern)
    _optional(args, "action", request.action)
    if request.effect is not None:
        args["effect"] = request.effect.value
    if request.state is not None:
        args["state"] = request.state.value
    if request.limit:
        args["limit"] = request.limit
    _optional(args, "cursor", request.cursor)
    return request, args


def _check_args(
    request: AccessControlCheckRequest,
) -> tuple[AccessControlCheckRequest, dict[str, object]]:
    owner_ura = _required_text(request.owner_ura, "owner_ura")
    _user_id_from_user_ura(owner_ura, "owner_ura")
    principal_id = _principal_id(request.principal_kind, request.principal_ura, "")
    if not principal_id:
        raise _invalid("principal_ura is required")
    for field_name, value in {
        "callee_ura": request.callee_ura,
        "subject_ura": request.subject_ura,
        "ability_ura": request.ability_ura,
        "action": request.action,
    }.items():
        _required_text(value, field_name)
    args: dict[str, object] = {
        "owner_ura": owner_ura,
        "principal_kind": request.principal_kind.value,
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


def _permission_request_create_args(
    request: AccessControlPermissionRequestCreateRequest,
) -> tuple[AccessControlPermissionRequestCreateRequest, dict[str, object]]:
    projected = _normalize_permission_request(
        request.request, request.owner_ura, request.principal_ura
    )
    args: dict[str, object] = {
        "request": _permission_request_wire(projected),
        "owner_ura": projected.owner_ura,
    }
    _optional(args, "principal_ura", projected.principal_ura)
    _optional(args, "actor_ura", request.actor_ura)
    return (
        AccessControlPermissionRequestCreateRequest(
            call=request.call,
            request=projected,
            owner_ura=projected.owner_ura,
            principal_ura=projected.principal_ura,
            actor_ura=request.actor_ura.strip(),
        ),
        args,
    )


def _permission_request_resolve_args(
    request: AccessControlPermissionRequestResolveRequest,
) -> tuple[AccessControlPermissionRequestResolveRequest, dict[str, object]]:
    projected = _normalize_permission_request(
        request.request, request.owner_ura, request.principal_ura
    )
    args: dict[str, object] = {
        "request": _permission_request_wire(projected),
        "owner_ura": projected.owner_ura,
    }
    _optional(args, "principal_ura", projected.principal_ura)
    _optional(args, "actor_ura", request.actor_ura)
    created_grant = None
    if request.created_grant is not None:
        created_grant = _normalize_grant(
            request.created_grant, projected.owner_ura, projected.principal_ura
        )
        args["created_grant"] = _grant_wire(created_grant)
    authority_proof = None
    if request.authority_proof is not None:
        authority_proof = replace(
            request.authority_proof,
            owner_ura=request.authority_proof.owner_ura.strip() or projected.owner_ura,
            principal_kind=request.authority_proof.principal_kind
            or projected.principal_kind,
            principal_ura=request.authority_proof.principal_ura.strip()
            or projected.principal_ura,
        )
        args["authority_proof"] = _authority_proof_wire(authority_proof)
    return (
        AccessControlPermissionRequestResolveRequest(
            call=request.call,
            request=projected,
            created_grant=created_grant,
            authority_proof=authority_proof,
            owner_ura=projected.owner_ura,
            principal_ura=projected.principal_ura,
            actor_ura=request.actor_ura.strip(),
        ),
        args,
    )


def _permission_request_list_args(
    request: AccessControlPermissionRequestListRequest,
) -> tuple[AccessControlPermissionRequestListRequest, dict[str, object]]:
    owner_ura = _required_text(request.owner_ura, "owner_ura")
    _user_id_from_user_ura(owner_ura, "owner_ura")
    _principal_id(request.principal_kind, request.principal_ura, request.principal_id)
    args: dict[str, object] = {"owner_ura": owner_ura}
    if request.principal_kind is not None:
        args["principal_kind"] = request.principal_kind.value
    _optional(args, "principal_ura", request.principal_ura)
    _optional(args, "token_id", request.token_id)
    _optional(args, "status", request.status)
    if request.limit:
        args["limit"] = request.limit
    _optional(args, "cursor", request.cursor)
    return request, args


def _admission_explain_args(
    request: AccessControlAdmissionExplainRequest,
) -> tuple[AccessControlAdmissionExplainRequest, dict[str, object]]:
    observer_ura = _required_text(request.observer_ura, "observer_ura")
    args: dict[str, object] = {"observer_ura": observer_ura}
    _optional(args, "invocation_id", request.invocation_id)
    _optional(args, "trace_id", request.trace_id)
    _optional(args, "root_id", request.root_id)
    return (
        AccessControlAdmissionExplainRequest(
            call=request.call,
            observer_ura=observer_ura,
            invocation_id=request.invocation_id.strip(),
            trace_id=request.trace_id.strip(),
            root_id=request.root_id.strip(),
        ),
        args,
    )


def _normalize_grant(
    grant: AccessControlGrant, owner_ura: str, principal_ura: str
) -> AccessControlGrant:
    effective_owner = owner_ura.strip() or grant.owner_ura.strip()
    _user_id_from_user_ura(effective_owner, "owner_ura")
    effective_principal = principal_ura.strip() or grant.principal_ura.strip()
    effective_principal_id = _principal_id(
        grant.principal_kind, effective_principal, grant.principal_id
    )
    if not grant.grant_id.strip():
        raise _invalid("grant_id is required")
    if not grant.actions:
        raise _invalid("grant actions are required")
    return AccessControlGrant(
        grant_id=grant.grant_id.strip(),
        owner_ura=effective_owner,
        principal_kind=grant.principal_kind,
        principal_id=effective_principal_id,
        principal_ura=effective_principal,
        token_id=grant.token_id.strip(),
        token_class=grant.token_class.strip(),
        callee_ura=grant.callee_ura.strip(),
        ability_ura_pattern=grant.ability_ura_pattern.strip(),
        subject_ura_pattern=grant.subject_ura_pattern.strip(),
        actions=tuple(grant.actions),
        effect=grant.effect,
        lifetime=grant.lifetime.strip(),
        state=grant.state,
        created_by=grant.created_by.strip(),
        created_at=grant.created_at.strip(),
        updated_at=grant.updated_at.strip(),
        expires_at=grant.expires_at.strip(),
        review_required_after=grant.review_required_after.strip(),
        last_reviewed_at=grant.last_reviewed_at.strip(),
        last_used_at=grant.last_used_at.strip(),
        revoked_at=grant.revoked_at.strip(),
        revoked_by=grant.revoked_by.strip(),
        revocation_reason=grant.revocation_reason.strip(),
        reason=grant.reason.strip(),
        constraints=dict(grant.constraints),
        authority_proof_id=grant.authority_proof_id.strip(),
        source_request_id=grant.source_request_id.strip(),
        invocation_template=dict(grant.invocation_template),
    )


def _grant_wire(grant: AccessControlGrant) -> dict[str, object]:
    wire: dict[str, object] = {
        "grant_id": grant.grant_id,
        "owner_ura": grant.owner_ura,
        "principal_kind": grant.principal_kind.value,
        "principal_ura": grant.principal_ura,
        "actions": list(grant.actions),
        "effect": grant.effect.value,
        "state": grant.state.value,
        "created_by": grant.created_by,
        "constraints": dict(grant.constraints),
    }
    for key, value in {
        "token_id": grant.token_id,
        "token_class": grant.token_class,
        "callee_ura": grant.callee_ura,
        "ability_ura_pattern": grant.ability_ura_pattern,
        "subject_ura_pattern": grant.subject_ura_pattern,
        "lifetime": grant.lifetime,
        "created_at": grant.created_at,
        "updated_at": grant.updated_at,
        "expires_at": grant.expires_at,
        "review_required_after": grant.review_required_after,
        "last_reviewed_at": grant.last_reviewed_at,
        "last_used_at": grant.last_used_at,
        "revoked_at": grant.revoked_at,
        "revoked_by": grant.revoked_by,
        "revocation_reason": grant.revocation_reason,
        "reason": grant.reason,
        "authority_proof_id": grant.authority_proof_id,
        "source_request_id": grant.source_request_id,
    }.items():
        _optional(wire, key, value)
    if grant.invocation_template:
        wire["invocation_template"] = dict(grant.invocation_template)
    return wire


def _normalize_permission_request(
    request: AccessControlPermissionRequest, owner_ura: str, principal_ura: str
) -> AccessControlPermissionRequest:
    effective_owner = owner_ura.strip() or request.owner_ura.strip()
    _user_id_from_user_ura(effective_owner, "owner_ura")
    effective_principal = principal_ura.strip() or request.principal_ura.strip()
    effective_kind = request.principal_kind or AccessControlPrincipalKind.USER
    effective_principal_id = _principal_id(
        effective_kind, effective_principal, request.principal_id
    )
    if not request.request_id.strip():
        raise _invalid("request_id is required")
    for field_name, value in {
        "callee_ura": request.callee_ura,
        "subject_ura": request.subject_ura,
        "ability_ura": request.ability_ura,
        "action": request.action,
    }.items():
        _required_text(value, field_name)
    return AccessControlPermissionRequest(
        request_id=request.request_id.strip(),
        owner_ura=effective_owner,
        caller_ura=request.caller_ura.strip(),
        principal_kind=effective_kind,
        principal_id=effective_principal_id,
        principal_ura=effective_principal,
        token_id=request.token_id.strip(),
        token_class=request.token_class.strip(),
        callee_ura=request.callee_ura.strip(),
        subject_ura=request.subject_ura.strip(),
        ability_ura=request.ability_ura.strip(),
        action=request.action.strip(),
        nonce=request.nonce.strip(),
        canonical_hash=request.canonical_hash.strip(),
        requested_lifetimes=tuple(request.requested_lifetimes),
        status=request.status.strip(),
        created_at=request.created_at.strip(),
        expires_at=request.expires_at.strip(),
        resolver_ura=request.resolver_ura.strip(),
        resolved_lifetime=request.resolved_lifetime.strip(),
        created_grant_id=request.created_grant_id.strip(),
        authority_proof_id=request.authority_proof_id.strip(),
        resolved_at=request.resolved_at.strip(),
        decision_reason=request.decision_reason.strip(),
    )


def _permission_request_wire(request: AccessControlPermissionRequest) -> dict[str, object]:
    wire: dict[str, object] = {
        "request_id": request.request_id,
        "owner_ura": request.owner_ura,
        "principal_kind": (request.principal_kind or AccessControlPrincipalKind.USER).value,
        "principal_ura": request.principal_ura,
        "callee_ura": request.callee_ura,
        "subject_ura": request.subject_ura,
        "ability_ura": request.ability_ura,
        "action": request.action,
    }
    for key, value in {
        "caller_ura": request.caller_ura,
        "token_id": request.token_id,
        "token_class": request.token_class,
        "nonce": request.nonce,
        "canonical_hash": request.canonical_hash,
        "status": request.status,
        "created_at": request.created_at,
        "expires_at": request.expires_at,
        "resolver_ura": request.resolver_ura,
        "resolved_lifetime": request.resolved_lifetime,
        "created_grant_id": request.created_grant_id,
        "authority_proof_id": request.authority_proof_id,
        "resolved_at": request.resolved_at,
        "decision_reason": request.decision_reason,
    }.items():
        _optional(wire, key, value)
    if request.requested_lifetimes:
        wire["requested_lifetimes"] = list(request.requested_lifetimes)
    return wire


def _authority_proof_wire(proof: AccessControlAuthorityProof) -> dict[str, object]:
    if not proof.proof_id.strip():
        raise _invalid("authority_proof.proof_id is required")
    _user_id_from_user_ura(proof.owner_ura.strip(), "authority_proof.owner_ura")
    principal_kind = proof.principal_kind or AccessControlPrincipalKind.USER
    _principal_id(principal_kind, proof.principal_ura, proof.principal_id)
    wire: dict[str, object] = {
        "proof_id": proof.proof_id.strip(),
        "owner_ura": proof.owner_ura.strip(),
        "principal_kind": principal_kind.value,
        "principal_ura": proof.principal_ura.strip(),
    }
    for key, value in {
        "grant_id": proof.grant_id,
        "permission_request_id": proof.permission_request_id,
        "token_id": proof.token_id,
        "callee_ura": proof.callee_ura,
        "ability_ura": proof.ability_ura,
        "subject_ura": proof.subject_ura,
        "action": proof.action,
        "nonce": proof.nonce,
        "canonical_hash": proof.canonical_hash,
        "canonical_invocation_hash": proof.canonical_invocation_hash,
        "session_id": proof.session_id,
        "session_owner_ura": proof.session_owner_ura,
        "session_expires_at": proof.session_expires_at,
        "issuer_ura": proof.issuer_ura,
        "audience_ura": proof.audience_ura,
        "issued_at": proof.issued_at,
        "expires_at": proof.expires_at,
        "signature": proof.signature,
        "verification_key_id": proof.verification_key_id,
    }.items():
        _optional(wire, key, value)
    if proof.allowed_followup_abilities:
        wire["allowed_followup_abilities"] = list(proof.allowed_followup_abilities)
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
        if principal_id:
            raise _invalid("principal_ura is required when principal_id is provided")
        return ""
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
        principal_id=_text(raw.get("principal_id")),
        principal_ura=_text(raw.get("principal_ura")),
        token_id=_text(raw.get("token_id")),
        token_class=_text(raw.get("token_class")),
        callee_ura=_text(raw.get("callee_ura")),
        ability_ura_pattern=_text(raw.get("ability_ura_pattern")),
        subject_ura_pattern=_text(raw.get("subject_ura_pattern")),
        actions=_string_tuple(raw.get("actions")),
        effect=_effect(raw.get("effect")),
        lifetime=_text(raw.get("lifetime")),
        state=_grant_state(raw.get("state")),
        created_by=_text(raw.get("created_by")),
        created_at=_text(raw.get("created_at")),
        updated_at=_text(raw.get("updated_at")),
        expires_at=_text(raw.get("expires_at")),
        review_required_after=_text(raw.get("review_required_after")),
        last_reviewed_at=_text(raw.get("last_reviewed_at")),
        last_used_at=_text(raw.get("last_used_at")),
        revoked_at=_text(raw.get("revoked_at")),
        revoked_by=_text(raw.get("revoked_by")),
        revocation_reason=_text(raw.get("revocation_reason")),
        reason=_text(raw.get("reason")),
        constraints=_mapping_or_empty(raw.get("constraints")),
        authority_proof_id=_text(raw.get("authority_proof_id")),
        source_request_id=_text(raw.get("source_request_id")),
        invocation_template=_mapping_or_empty(raw.get("invocation_template")),
    )


def _permission_request_from_mapping(
    raw: Mapping[str, object]
) -> AccessControlPermissionRequest:
    request_id = _required_text(_text(raw.get("request_id")), "request_id")
    return AccessControlPermissionRequest(
        request_id=request_id,
        owner_ura=_text(raw.get("owner_ura")),
        caller_ura=_text(raw.get("caller_ura")),
        principal_kind=_optional_principal_kind(raw.get("principal_kind")),
        principal_id=_text(raw.get("principal_id")),
        principal_ura=_text(raw.get("principal_ura")),
        token_id=_text(raw.get("token_id")),
        token_class=_text(raw.get("token_class")),
        callee_ura=_text(raw.get("callee_ura")),
        subject_ura=_text(raw.get("subject_ura")),
        ability_ura=_text(raw.get("ability_ura")),
        action=_text(raw.get("action")),
        nonce=_text(raw.get("nonce")),
        canonical_hash=_text(raw.get("canonical_hash")),
        requested_lifetimes=_string_tuple(raw.get("requested_lifetimes")),
        status=_text(raw.get("status")),
        created_at=_text(raw.get("created_at")),
        expires_at=_text(raw.get("expires_at")),
        resolver_ura=_text(raw.get("resolver_ura")),
        resolved_lifetime=_text(raw.get("resolved_lifetime")),
        created_grant_id=_text(raw.get("created_grant_id")),
        authority_proof_id=_text(raw.get("authority_proof_id")),
        resolved_at=_text(raw.get("resolved_at")),
        decision_reason=_text(raw.get("decision_reason")),
    )


def _authority_proof_from_mapping(
    raw: Mapping[str, object]
) -> AccessControlAuthorityProof:
    proof_id = _required_text(_text(raw.get("proof_id")), "proof_id")
    return AccessControlAuthorityProof(
        proof_id=proof_id,
        grant_id=_text(raw.get("grant_id")),
        permission_request_id=_text(raw.get("permission_request_id")),
        owner_ura=_text(raw.get("owner_ura")),
        principal_kind=_optional_principal_kind(raw.get("principal_kind")),
        principal_id=_text(raw.get("principal_id")),
        principal_ura=_text(raw.get("principal_ura")),
        token_id=_text(raw.get("token_id")),
        callee_ura=_text(raw.get("callee_ura")),
        ability_ura=_text(raw.get("ability_ura")),
        subject_ura=_text(raw.get("subject_ura")),
        action=_text(raw.get("action")),
        nonce=_text(raw.get("nonce")),
        canonical_hash=_text(raw.get("canonical_hash")),
        canonical_invocation_hash=_text(raw.get("canonical_invocation_hash")),
        session_id=_text(raw.get("session_id")),
        session_owner_ura=_text(raw.get("session_owner_ura")),
        allowed_followup_abilities=_string_tuple(
            raw.get("allowed_followup_abilities")
        ),
        session_expires_at=_text(raw.get("session_expires_at")),
        audience_ura=_text(raw.get("audience_ura")),
        issuer_ura=_text(raw.get("issuer_ura")),
        issued_at=_text(raw.get("issued_at")),
        expires_at=_text(raw.get("expires_at")),
        signature=_text(raw.get("signature")),
        verification_key_id=_text(raw.get("verification_key_id")),
    )


def _policy_decision_from_mapping(raw: Mapping[str, object]) -> AccessControlPolicyDecision:
    decision = _required_text(_text(raw.get("decision")), "decision")
    return AccessControlPolicyDecision(
        decision=decision,
        reason=_text(raw.get("reason")),
        owner_user_id=_text(raw.get("owner_user_id")),
        owner_ura=_text(raw.get("owner_ura")),
        owner_source=_text(raw.get("owner_source")),
        caller_ura=_text(raw.get("caller_ura")),
        principal_kind=_optional_principal_kind(raw.get("principal_kind")),
        principal_id=_text(raw.get("principal_id")),
        principal_ura=_text(raw.get("principal_ura")),
        token_id=_text(raw.get("token_id")),
        callee_ura=_text(raw.get("callee_ura")),
        ability_ura=_text(raw.get("ability_ura")),
        subject_ura=_text(raw.get("subject_ura")),
        action=_text(raw.get("action")),
        grant_id=_text(raw.get("grant_id")),
        policy_rule_id=_text(raw.get("policy_rule_id")),
        prompt_request_id=_text(raw.get("prompt_request_id")),
        canonical_hash=_text(raw.get("canonical_hash")),
        signature_key_id=_text(raw.get("signature_key_id")),
        rejector_ura=_text(raw.get("rejector_ura")),
        authority_proof_id=_text(raw.get("authority_proof_id")),
        audit_warnings=_string_tuple(raw.get("audit_warnings")),
    )


def _signature_decision_from_mapping(
    raw: Mapping[str, object]
) -> AccessControlSignatureDecision:
    decision = _required_text(_text(raw.get("decision")), "decision")
    return AccessControlSignatureDecision(
        decision=decision,
        reason=_text(raw.get("reason")),
        caller_ura=_text(raw.get("caller_ura")),
        callee_ura=_text(raw.get("callee_ura")),
        ability_ura=_text(raw.get("ability_ura")),
        subject_ura=_text(raw.get("subject_ura")),
        canonical_hash=_text(raw.get("canonical_hash")),
        signature_key_id=_text(raw.get("signature_key_id")),
        presented_pubkey_fingerprint=_text(raw.get("presented_pubkey_fingerprint")),
        verifier_ura=_text(raw.get("verifier_ura")),
        rejector_ura=_text(raw.get("rejector_ura")),
    )


def _ability_call_trace_from_mapping(
    raw: Mapping[str, object]
) -> AccessControlAbilityCallTrace:
    signature_raw = raw.get("signature_decision")
    policy_raw = raw.get("policy_decision")
    children_raw = raw.get("children")
    children: tuple[AccessControlAbilityCallTrace, ...] = ()
    if isinstance(children_raw, Sequence) and not isinstance(
        children_raw, (str, bytes, bytearray)
    ):
        children = tuple(
            _ability_call_trace_from_mapping(_mapping(item, "child_trace"))
            for item in children_raw
        )
    return AccessControlAbilityCallTrace(
        invocation_id=_text(raw.get("invocation_id")),
        parent_invocation_id=_text(raw.get("parent_invocation_id")),
        root_invocation_id=_text(raw.get("root_invocation_id")),
        caller_ura=_text(raw.get("caller_ura")),
        callee_ura=_text(raw.get("callee_ura")),
        subject_ura=_text(raw.get("subject_ura")),
        ability_ura=_text(raw.get("ability_ura")),
        action=_text(raw.get("action")),
        route_ref=_text(raw.get("route_ref")),
        execution_host_ura=_text(raw.get("execution_host_ura")),
        rejector_ura=_text(raw.get("rejector_ura")),
        stage=_text(raw.get("stage")),
        signature_decision=(
            _signature_decision_from_mapping(
                _mapping(signature_raw, "signature_decision")
            )
            if isinstance(signature_raw, Mapping)
            else None
        ),
        policy_decision=(
            _policy_decision_from_mapping(_mapping(policy_raw, "policy_decision"))
            if isinstance(policy_raw, Mapping)
            else None
        ),
        authority_proof_id=_text(raw.get("authority_proof_id")),
        redacted=_bool(raw.get("redacted")),
        child_failure_class=_text(raw.get("child_failure_class")),
        redaction_reason=_text(raw.get("redaction_reason")),
        children=children,
    )


def _admission_explain_from_mapping(
    raw: Mapping[str, object]
) -> AccessControlAdmissionExplainResult:
    root_trace_raw = raw.get("root_trace")
    signature_raw = raw.get("signature_decision")
    policy_raw = raw.get("policy_decision")
    return AccessControlAdmissionExplainResult(
        observer_ura=_text(raw.get("observer_ura")),
        redacted=_bool(raw.get("redacted")),
        root_trace=(
            _ability_call_trace_from_mapping(_mapping(root_trace_raw, "root_trace"))
            if isinstance(root_trace_raw, Mapping)
            else None
        ),
        signature_decision=(
            _signature_decision_from_mapping(
                _mapping(signature_raw, "signature_decision")
            )
            if isinstance(signature_raw, Mapping)
            else None
        ),
        policy_decision=(
            _policy_decision_from_mapping(_mapping(policy_raw, "policy_decision"))
            if isinstance(policy_raw, Mapping)
            else None
        ),
        authority_reason=_text(raw.get("authority_reason")),
        route_ref=_text(raw.get("route_ref")),
        rejector_ura=_text(raw.get("rejector_ura")),
        redaction_reason=_text(raw.get("redaction_reason")),
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


def _bool(value: object) -> bool:
    return value if isinstance(value, bool) else False


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
