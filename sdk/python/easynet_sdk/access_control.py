"""RFC-014 access-control DTOs and typed client facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Mapping, Protocol

from .errors import ErrorCode, RetryHint, SDKError


class PrincipalKind(StrEnum):
    USER = "user"
    TOKEN = "token"
    HUB = "hub"
    DEVICE = "device"
    SERVICE = "service"
    AUTOMATION = "automation"


class TokenClass(StrEnum):
    HUB_LINK = "hub_link"
    BROWSER_SESSION = "browser_session"
    DEVICE_PAIRING = "device_pairing"
    AUTOMATION = "automation"
    THIRD_PARTY = "third_party"
    SERVICE = "service"


class AccessAction(StrEnum):
    READ = "read"
    INVOKE = "invoke"
    STREAM = "stream"
    MANAGE = "manage"
    GRANT = "grant"


@dataclass(frozen=True)
class PermissionGrant:
    grant_id: str
    owner_user_id: str
    principal_kind: str
    principal_id: str
    actions: tuple[str, ...]
    effect: str
    lifetime: str
    state: str
    created_by: str
    created_at: str
    token_id: str | None = None
    token_class: str | None = None
    callee_ura: str | None = None
    subject_ura_pattern: str | None = None
    ability_ura_pattern: str | None = None
    constraints: Mapping[str, object] | None = None
    expires_at: str | None = None
    review_required_after: str | None = None
    last_reviewed_at: str | None = None
    last_used_at: str | None = None
    updated_at: str | None = None
    revoked_at: str | None = None
    reason: str | None = None

    def to_dict(self) -> dict[str, object]:
        return _drop_none(
            {
                "grant_id": self.grant_id,
                "owner_user_id": self.owner_user_id,
                "principal_kind": self.principal_kind,
                "principal_id": self.principal_id,
                "token_id": self.token_id,
                "token_class": self.token_class,
                "callee_ura": self.callee_ura,
                "subject_ura_pattern": self.subject_ura_pattern,
                "ability_ura_pattern": self.ability_ura_pattern,
                "actions": list(self.actions),
                "constraints": dict(self.constraints) if self.constraints else None,
                "effect": self.effect,
                "lifetime": self.lifetime,
                "state": self.state,
                "expires_at": self.expires_at,
                "review_required_after": self.review_required_after,
                "last_reviewed_at": self.last_reviewed_at,
                "last_used_at": self.last_used_at,
                "created_by": self.created_by,
                "created_at": self.created_at,
                "updated_at": self.updated_at,
                "revoked_at": self.revoked_at,
                "reason": self.reason,
            }
        )

    @classmethod
    def from_dict(cls, raw: Mapping[str, object]) -> "PermissionGrant":
        return cls(
            grant_id=_required_string(raw, "grant_id"),
            owner_user_id=_required_string(raw, "owner_user_id"),
            principal_kind=_required_string(raw, "principal_kind"),
            principal_id=_required_string(raw, "principal_id"),
            actions=tuple(_required_list(raw, "actions")),
            effect=_required_string(raw, "effect"),
            lifetime=_required_string(raw, "lifetime"),
            state=_required_string(raw, "state"),
            created_by=_required_string(raw, "created_by"),
            created_at=_required_string(raw, "created_at"),
            token_id=_optional_string(raw, "token_id"),
            token_class=_optional_string(raw, "token_class"),
            callee_ura=_optional_string(raw, "callee_ura"),
            subject_ura_pattern=_optional_string(raw, "subject_ura_pattern"),
            ability_ura_pattern=_optional_string(raw, "ability_ura_pattern"),
            constraints=_optional_mapping(raw, "constraints"),
            expires_at=_optional_string(raw, "expires_at"),
            review_required_after=_optional_string(raw, "review_required_after"),
            last_reviewed_at=_optional_string(raw, "last_reviewed_at"),
            last_used_at=_optional_string(raw, "last_used_at"),
            updated_at=_optional_string(raw, "updated_at"),
            revoked_at=_optional_string(raw, "revoked_at"),
            reason=_optional_string(raw, "reason"),
        )


@dataclass(frozen=True)
class PolicyDecision:
    raw: Mapping[str, object]

    @property
    def decision(self) -> str:
        return _required_string(self.raw, "decision")

    @property
    def reason(self) -> str:
        return _required_string(self.raw, "reason")


@dataclass(frozen=True)
class SignatureDecision:
    raw: Mapping[str, object]

    @property
    def decision(self) -> str:
        return _required_string(self.raw, "decision")

    @property
    def reason(self) -> str:
        return _required_string(self.raw, "reason")


@dataclass(frozen=True)
class AuthorityProof:
    raw: Mapping[str, object]

    @property
    def proof_id(self) -> str:
        return _required_string(self.raw, "proof_id")

    def to_dict(self) -> dict[str, object]:
        return dict(self.raw)

    @classmethod
    def from_dict(cls, raw: Mapping[str, object]) -> "AuthorityProof":
        _required_string(raw, "proof_id")
        _required_string(raw, "owner_user_id")
        return cls(dict(raw))


@dataclass(frozen=True)
class AbilityCallTrace:
    raw: Mapping[str, object]

    @property
    def invocation_id(self) -> str:
        return _required_string(self.raw, "invocation_id")

    @property
    def redacted(self) -> bool:
        return bool(self.raw.get("redacted", False))


@dataclass(frozen=True)
class PermissionRequest:
    raw: Mapping[str, object]

    @property
    def request_id(self) -> str:
        return _required_string(self.raw, "request_id")

    def to_dict(self) -> dict[str, object]:
        return dict(self.raw)

    @classmethod
    def from_dict(cls, raw: Mapping[str, object]) -> "PermissionRequest":
        _required_string(raw, "request_id")
        _required_string(raw, "owner_user_id")
        return cls(dict(raw))


@dataclass(frozen=True)
class AdmissionExplainResult:
    observer_ura: str
    redacted: bool
    raw: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_dict(cls, raw: Mapping[str, object]) -> "AdmissionExplainResult":
        return cls(
            observer_ura=_required_string(raw, "observer_ura"),
            redacted=bool(raw.get("redacted", False)),
            raw=dict(raw),
        )


@dataclass(frozen=True)
class AuthorityBindingGrantResult:
    grant: PermissionGrant
    idempotent_replay: bool
    audit_record_id: str

    @classmethod
    def from_dict(cls, raw: Mapping[str, object]) -> "AuthorityBindingGrantResult":
        grant = raw.get("grant")
        if not isinstance(grant, Mapping):
            raise _invalid_access_control("grant result must carry grant object")
        return cls(
            grant=PermissionGrant.from_dict(grant),
            idempotent_replay=bool(raw.get("idempotent_replay", False)),
            audit_record_id=_required_string(raw, "audit_record_id"),
        )


@dataclass(frozen=True)
class PermissionRequestResolutionResult:
    request: PermissionRequest
    created_grant: PermissionGrant | None = None
    authority_proof: AuthorityProof | None = None
    idempotent_replay: bool = False

    @classmethod
    def from_dict(cls, raw: Mapping[str, object]) -> "PermissionRequestResolutionResult":
        request = PermissionRequest.from_dict(_required_mapping(raw, "request"))
        grant = raw.get("created_grant")
        proof = raw.get("authority_proof")
        if grant is not None and not isinstance(grant, Mapping):
            raise _invalid_access_control("created_grant must be an object")
        if proof is not None and not isinstance(proof, Mapping):
            raise _invalid_access_control("authority_proof must be an object")
        return cls(
            request=request,
            created_grant=PermissionGrant.from_dict(grant) if isinstance(grant, Mapping) else None,
            authority_proof=AuthorityProof.from_dict(proof) if isinstance(proof, Mapping) else None,
            idempotent_replay=bool(raw.get("idempotent_replay", False)),
        )


class AccessControlTransport(Protocol):
    def grant_authority_binding(self, request_json: bytes) -> bytes: ...
    def revoke_authority_binding(self, request_json: bytes) -> bytes: ...
    def list_authority_bindings(self, request_json: bytes) -> bytes: ...
    def check_authority_binding(self, request_json: bytes) -> bytes: ...
    def create_policy_request(self, request_json: bytes) -> bytes: ...
    def resolve_policy_request(self, request_json: bytes) -> bytes: ...
    def list_policy_requests(self, request_json: bytes) -> bytes: ...
    def explain_admission(self, request_json: bytes) -> bytes: ...


class AccessControlClient:
    def __init__(self, transport: AccessControlTransport):
        if transport is None:
            raise _invalid_access_control("access-control transport is required")
        self._transport = transport

    def grant(self, grant: PermissionGrant, *, actor_ura: str = "") -> AuthorityBindingGrantResult:
        raw = self._transport.grant_authority_binding(
            _json_bytes({"grant": grant.to_dict(), "actor_ura": actor_ura})
        )
        return AuthorityBindingGrantResult.from_dict(_json_object(raw))

    def revoke(
        self, *, owner_user_id: str, grant_id: str, actor_ura: str = "", reason: str = ""
    ) -> PermissionGrant:
        raw = self._transport.revoke_authority_binding(
            _json_bytes(
                {
                    "owner_user_id": owner_user_id,
                    "grant_id": grant_id,
                    "actor_ura": actor_ura,
                    "reason": reason,
                }
            )
        )
        decoded = _json_object(raw)
        grant = decoded.get("grant")
        if not isinstance(grant, Mapping):
            raise _invalid_access_control("revoke response must carry grant")
        return PermissionGrant.from_dict(grant)

    def list_grants(self, request: Mapping[str, object]) -> tuple[PermissionGrant, ...]:
        decoded = _json_object(self._transport.list_authority_bindings(_json_bytes(request)))
        grants = decoded.get("grants", [])
        if not isinstance(grants, list):
            raise _invalid_access_control("grants must be a list")
        return tuple(PermissionGrant.from_dict(item) for item in grants if isinstance(item, Mapping))

    def check(self, request: Mapping[str, object]) -> PolicyDecision:
        decoded = _json_object(self._transport.check_authority_binding(_json_bytes(request)))
        decision = decoded.get("policy_decision")
        if not isinstance(decision, Mapping):
            raise _invalid_access_control("check response must carry policy_decision")
        return PolicyDecision(dict(decision))

    def create_request(self, request: PermissionRequest, *, actor_ura: str = "") -> PermissionRequest:
        decoded = _json_object(
            self._transport.create_policy_request(
                _json_bytes({"request": request.to_dict(), "actor_ura": actor_ura})
            )
        )
        return PermissionRequest.from_dict(_required_mapping(decoded, "request"))

    def resolve_request(self, request: PermissionRequest, *, actor_ura: str = "") -> PermissionRequest:
        return self.resolve_request_result(request, actor_ura=actor_ura).request

    def resolve_request_result(
        self, request: PermissionRequest, *, actor_ura: str = ""
    ) -> PermissionRequestResolutionResult:
        decoded = _json_object(
            self._transport.resolve_policy_request(
                _json_bytes({"request": request.to_dict(), "actor_ura": actor_ura})
            )
        )
        return PermissionRequestResolutionResult.from_dict(decoded)

    def resolve_request_with_grant(
        self,
        request: PermissionRequest,
        grant: PermissionGrant,
        *,
        actor_ura: str = "",
    ) -> PermissionRequestResolutionResult:
        decoded = _json_object(
            self._transport.resolve_policy_request(
                _json_bytes(
                    {
                        "request": request.to_dict(),
                        "created_grant": grant.to_dict(),
                        "actor_ura": actor_ura,
                    }
                )
            )
        )
        return PermissionRequestResolutionResult.from_dict(decoded)

    def resolve_request_with_authority_proof(
        self,
        request: PermissionRequest,
        authority_proof: AuthorityProof,
        *,
        actor_ura: str = "",
    ) -> PermissionRequestResolutionResult:
        decoded = _json_object(
            self._transport.resolve_policy_request(
                _json_bytes(
                    {
                        "request": request.to_dict(),
                        "authority_proof": authority_proof.to_dict(),
                        "actor_ura": actor_ura,
                    }
                )
            )
        )
        return PermissionRequestResolutionResult.from_dict(decoded)

    def list_requests(self, request: Mapping[str, object]) -> tuple[PermissionRequest, ...]:
        decoded = _json_object(self._transport.list_policy_requests(_json_bytes(request)))
        requests = decoded.get("requests", [])
        if not isinstance(requests, list):
            raise _invalid_access_control("requests must be a list")
        return tuple(PermissionRequest.from_dict(item) for item in requests if isinstance(item, Mapping))

    def explain(self, request: Mapping[str, object]) -> AdmissionExplainResult:
        return AdmissionExplainResult.from_dict(
            _json_object(self._transport.explain_admission(_json_bytes(request)))
        )


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(dict(value), separators=(",", ":")).encode("utf-8")


def _json_object(raw: bytes | str) -> Mapping[str, object]:
    text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
    try:
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_access_control(f"decode access-control JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_access_control("access-control JSON must be an object")
    return decoded


def _drop_none(value: Mapping[str, object | None]) -> dict[str, object]:
    return {key: item for key, item in value.items() if item is not None}


def _required_string(raw: Mapping[str, object], key: str) -> str:
    value = raw.get(key)
    if not isinstance(value, str) or not value:
        raise _invalid_access_control(f"{key} is required")
    return value


def _optional_string(raw: Mapping[str, object], key: str) -> str | None:
    value = raw.get(key)
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_access_control(f"{key} must be a string")
    return value


def _required_list(raw: Mapping[str, object], key: str) -> list[str]:
    value = raw.get(key)
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise _invalid_access_control(f"{key} must be a string list")
    return list(value)


def _optional_mapping(raw: Mapping[str, object], key: str) -> Mapping[str, object] | None:
    value = raw.get(key)
    if value is None:
        return None
    if not isinstance(value, Mapping):
        raise _invalid_access_control(f"{key} must be an object")
    return dict(value)


def _required_mapping(raw: Mapping[str, object], key: str) -> Mapping[str, object]:
    value = raw.get(key)
    if not isinstance(value, Mapping):
        raise _invalid_access_control(f"{key} must be an object")
    return dict(value)


def _invalid_access_control(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="access_control",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
        details={"profile": "access_control"},
    )
