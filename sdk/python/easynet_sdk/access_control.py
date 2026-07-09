"""RFC-014 access-control DTOs and typed client facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Any, Mapping, Protocol

from .errors import ErrorCode, RetryHint, SDKError, canonical_failure_code
from .identity import IdentityClient
from .invocation import InvocationDraft
from .runtime import RuntimeClient
from .system_abilities import AccessControlSystemAbility


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


@dataclass(frozen=True)
class AccessControlCarrierBase:
    """Complete Runtime Core carrier context for access-control system abilities."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_dict(self) -> dict[str, object]:
        _validate_carrier(self)
        value: dict[str, object] = {
            "caller_ura": self.caller_ura,
            "callee_ura": self.callee_ura,
            "subject_ura": self.subject_ura,
            "descriptor_version": self.descriptor_version,
            "nonce_base64": self.nonce_base64,
            "causal_context": dict(self.causal_context),
        }
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return value


@dataclass(frozen=True)
class AuthorityBindingGrantRequest:
    carrier: AccessControlCarrierBase
    grant: PermissionGrant
    actor_ura: str

    def to_json_dict(self) -> dict[str, object]:
        return {
            "carrier": self.carrier.to_json_dict(),
            "grant": self.grant.to_dict(),
            "actor_ura": self.actor_ura,
        }


@dataclass(frozen=True)
class AuthorityBindingRevokeRequest:
    carrier: AccessControlCarrierBase
    owner_user_id: str
    grant_id: str
    actor_ura: str = ""
    reason: str = ""

    def to_json_dict(self) -> dict[str, object]:
        return _drop_none(
            {
                "carrier": self.carrier.to_json_dict(),
                "owner_user_id": self.owner_user_id,
                "grant_id": self.grant_id,
                "actor_ura": self.actor_ura,
                "reason": self.reason or None,
            }
        )


@dataclass(frozen=True)
class AuthorityBindingListRequest:
    carrier: AccessControlCarrierBase
    owner_user_id: str = ""
    principal_kind: str = ""
    principal_id: str = ""
    token_id: str = ""
    callee_ura: str = ""
    subject_ura: str = ""
    ability_ura: str = ""
    action: str = ""
    state: str = ""
    limit: int = 0
    cursor: str = ""

    def to_json_dict(self) -> dict[str, object]:
        return _drop_empty(
            {
                "carrier": self.carrier.to_json_dict(),
                "owner_user_id": self.owner_user_id,
                "principal_kind": self.principal_kind,
                "principal_id": self.principal_id,
                "token_id": self.token_id,
                "callee_ura": self.callee_ura,
                "subject_ura": self.subject_ura,
                "ability_ura": self.ability_ura,
                "action": self.action,
                "state": self.state,
                "limit": self.limit,
                "cursor": self.cursor,
            }
        )


@dataclass(frozen=True)
class AuthorityBindingCheckRequest:
    carrier: AccessControlCarrierBase
    caller_ura: str
    principal_kind: str
    principal_id: str
    callee_ura: str
    subject_ura: str
    ability_ura: str
    action: str
    token_id: str = ""
    canonical_hash: str = ""
    signature_key_id: str = ""

    def to_json_dict(self) -> dict[str, object]:
        return _drop_empty(
            {
                "carrier": self.carrier.to_json_dict(),
                "caller_ura": self.caller_ura,
                "principal_kind": self.principal_kind,
                "principal_id": self.principal_id,
                "token_id": self.token_id,
                "callee_ura": self.callee_ura,
                "subject_ura": self.subject_ura,
                "ability_ura": self.ability_ura,
                "action": self.action,
                "canonical_hash": self.canonical_hash,
                "signature_key_id": self.signature_key_id,
            }
        )


@dataclass(frozen=True)
class PolicyRequestCreateRequest:
    carrier: AccessControlCarrierBase
    request: PermissionRequest
    actor_ura: str

    def to_json_dict(self) -> dict[str, object]:
        return {
            "carrier": self.carrier.to_json_dict(),
            "request": self.request.to_dict(),
            "actor_ura": self.actor_ura,
        }


@dataclass(frozen=True)
class PolicyRequestResolveRequest:
    carrier: AccessControlCarrierBase
    request: PermissionRequest
    actor_ura: str
    created_grant: PermissionGrant | None = None
    authority_proof: AuthorityProof | None = None

    def to_json_dict(self) -> dict[str, object]:
        return _drop_none(
            {
                "carrier": self.carrier.to_json_dict(),
                "request": self.request.to_dict(),
                "created_grant": self.created_grant.to_dict()
                if self.created_grant is not None
                else None,
                "authority_proof": self.authority_proof.to_dict()
                if self.authority_proof is not None
                else None,
                "actor_ura": self.actor_ura,
            }
        )


@dataclass(frozen=True)
class PolicyRequestListRequest:
    carrier: AccessControlCarrierBase
    owner_user_id: str = ""
    principal_kind: str = ""
    principal_id: str = ""
    token_id: str = ""
    status: str = ""
    limit: int = 0
    cursor: str = ""

    def to_json_dict(self) -> dict[str, object]:
        return _drop_empty(
            {
                "carrier": self.carrier.to_json_dict(),
                "owner_user_id": self.owner_user_id,
                "principal_kind": self.principal_kind,
                "principal_id": self.principal_id,
                "token_id": self.token_id,
                "status": self.status,
                "limit": self.limit,
                "cursor": self.cursor,
            }
        )


@dataclass(frozen=True)
class AdmissionExplainRequest:
    carrier: AccessControlCarrierBase
    observer_ura: str
    invocation_id: str = ""
    trace_id: str = ""
    root_id: str = ""

    def to_json_dict(self) -> dict[str, object]:
        return _drop_empty(
            {
                "carrier": self.carrier.to_json_dict(),
                "observer_ura": self.observer_ura,
                "invocation_id": self.invocation_id,
                "trace_id": self.trace_id,
                "root_id": self.root_id,
            }
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


@dataclass
class RuntimeAccessControlTransport:
    """Access-control transport that executes RFC-014 operations through Runtime Core."""

    runtime: RuntimeClient
    identity: IdentityClient
    _closed: bool = field(default=False, init=False, repr=False)

    def grant_authority_binding(self, request_json: bytes) -> bytes:
        return self._invoke(request_json, AccessControlSystemAbility.AUTHORITY_BINDING_GRANT)

    def revoke_authority_binding(self, request_json: bytes) -> bytes:
        return self._invoke(request_json, AccessControlSystemAbility.AUTHORITY_BINDING_REVOKE)

    def list_authority_bindings(self, request_json: bytes) -> bytes:
        return self._invoke(request_json, AccessControlSystemAbility.AUTHORITY_BINDING_LIST)

    def check_authority_binding(self, request_json: bytes) -> bytes:
        return self._invoke(request_json, AccessControlSystemAbility.AUTHORITY_BINDING_CHECK)

    def create_policy_request(self, request_json: bytes) -> bytes:
        return self._invoke(request_json, AccessControlSystemAbility.POLICY_REQUEST_CREATE)

    def resolve_policy_request(self, request_json: bytes) -> bytes:
        return self._invoke(request_json, AccessControlSystemAbility.POLICY_REQUEST_RESOLVE)

    def list_policy_requests(self, request_json: bytes) -> bytes:
        return self._invoke(request_json, AccessControlSystemAbility.POLICY_REQUEST_LIST)

    def explain_admission(self, request_json: bytes) -> bytes:
        return self._invoke(request_json, AccessControlSystemAbility.ADMISSION_EXPLAIN)

    def close(self) -> None:
        self._closed = True

    def _invoke(self, request_json: bytes, ability: AccessControlSystemAbility) -> bytes:
        self._require_open()
        draft = self._build_invocation(request_json, ability.value)
        result = self.runtime.invoke(draft)
        if not result.ok:
            raise SDKError(
                code=canonical_failure_code(result.error.code if result.error else None),
                stage="access_control",
                retry=RetryHint.UNKNOWN,
                retryable=False,
                message="access-control invocation failed",
                cause=result.error,
                details={"profile": "access_control"},
            )
        if result.output_json is None:
            raise _invalid_access_control("access-control output_json is required")
        return _json_bytes(_output_mapping(result.output_json))

    def _build_invocation(self, request_json: bytes, ability_name: str) -> InvocationDraft:
        carrier, args = _runtime_args(request_json)
        descriptor_ref = self.identity.owner_ability_descriptor_ref(
            carrier.callee_ura,
            ability_name,
            carrier.descriptor_version,
        )
        subject_ura = _descriptor_bound_subject_ura(
            self.identity,
            carrier.subject_ura,
            ability_name,
        )
        return InvocationDraft.from_json(
            _json_bytes(
                {
                    "caller_ura": carrier.caller_ura,
                    "callee_ura": carrier.callee_ura,
                    "descriptor_ref": descriptor_ref,
                    "subject_ura": subject_ura,
                    "nonce_base64": carrier.nonce_base64,
                    "causal_context": dict(carrier.causal_context),
                    "args": args,
                    "content_type": "application/json",
                    "metadata": _runtime_metadata(carrier.metadata, ability_name),
                }
            )
        )

    def _require_open(self) -> None:
        if self._closed:
            raise _invalid_access_control("access-control runtime transport is closed")
        if self.runtime is None:
            raise _invalid_access_control("runtime client is required")
        if self.identity is None:
            raise _invalid_access_control("identity client is required")


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

    def grant_with_request(
        self, request: AuthorityBindingGrantRequest
    ) -> AuthorityBindingGrantResult:
        raw = self._transport.grant_authority_binding(_json_bytes(request.to_json_dict()))
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

    def revoke_with_request(self, request: AuthorityBindingRevokeRequest) -> PermissionGrant:
        decoded = _json_object(
            self._transport.revoke_authority_binding(_json_bytes(request.to_json_dict()))
        )
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

    def list_grants_with_request(
        self, request: AuthorityBindingListRequest
    ) -> tuple[PermissionGrant, ...]:
        decoded = _json_object(
            self._transport.list_authority_bindings(_json_bytes(request.to_json_dict()))
        )
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

    def check_with_request(self, request: AuthorityBindingCheckRequest) -> PolicyDecision:
        decoded = _json_object(
            self._transport.check_authority_binding(_json_bytes(request.to_json_dict()))
        )
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

    def create_request_with_carrier(
        self, request: PolicyRequestCreateRequest
    ) -> PermissionRequest:
        decoded = _json_object(
            self._transport.create_policy_request(_json_bytes(request.to_json_dict()))
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

    def resolve_request_with_carrier(
        self, request: PolicyRequestResolveRequest
    ) -> PermissionRequestResolutionResult:
        decoded = _json_object(
            self._transport.resolve_policy_request(_json_bytes(request.to_json_dict()))
        )
        return PermissionRequestResolutionResult.from_dict(decoded)

    def list_requests(self, request: Mapping[str, object]) -> tuple[PermissionRequest, ...]:
        decoded = _json_object(self._transport.list_policy_requests(_json_bytes(request)))
        requests = decoded.get("requests", [])
        if not isinstance(requests, list):
            raise _invalid_access_control("requests must be a list")
        return tuple(PermissionRequest.from_dict(item) for item in requests if isinstance(item, Mapping))

    def list_requests_with_request(
        self, request: PolicyRequestListRequest
    ) -> tuple[PermissionRequest, ...]:
        decoded = _json_object(
            self._transport.list_policy_requests(_json_bytes(request.to_json_dict()))
        )
        requests = decoded.get("requests", [])
        if not isinstance(requests, list):
            raise _invalid_access_control("requests must be a list")
        return tuple(PermissionRequest.from_dict(item) for item in requests if isinstance(item, Mapping))

    def explain(self, request: Mapping[str, object]) -> AdmissionExplainResult:
        return AdmissionExplainResult.from_dict(
            _json_object(self._transport.explain_admission(_json_bytes(request)))
        )

    def explain_with_request(self, request: AdmissionExplainRequest) -> AdmissionExplainResult:
        return AdmissionExplainResult.from_dict(
            _json_object(self._transport.explain_admission(_json_bytes(request.to_json_dict())))
        )

    def close(self) -> None:
        close = getattr(self._transport, "close", None)
        if callable(close):
            close()


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


def _drop_empty(value: Mapping[str, object]) -> dict[str, object]:
    return {
        key: item
        for key, item in value.items()
        if item != "" and item != 0 and item is not None
    }


def _runtime_args(request_json: bytes) -> tuple[AccessControlCarrierBase, dict[str, object]]:
    payload = dict(_json_object(request_json))
    raw_carrier = payload.pop("carrier", None)
    if not isinstance(raw_carrier, Mapping):
        raise _invalid_access_control("carrier is required for runtime access-control requests")
    carrier = AccessControlCarrierBase(
        caller_ura=_required_string(raw_carrier, "caller_ura"),
        callee_ura=_required_string(raw_carrier, "callee_ura"),
        subject_ura=_required_string(raw_carrier, "subject_ura"),
        descriptor_version=_required_string(raw_carrier, "descriptor_version"),
        nonce_base64=_required_string(raw_carrier, "nonce_base64"),
        causal_context=_required_mapping(raw_carrier, "causal_context"),
        metadata=_optional_mapping(raw_carrier, "metadata") or {},
    )
    _validate_carrier(carrier)
    return carrier, payload


def _runtime_metadata(metadata: Mapping[str, object], ability_name: str) -> dict[str, object]:
    value = dict(metadata)
    value["profile"] = "access_control"
    value["system_ability"] = ability_name
    value["carrier_owner"] = "daemon_sdk"
    return value


def _descriptor_bound_subject_ura(
    identity: IdentityClient, subject_ura: str, ability_name: str
) -> str:
    projection = identity.parse_ura(subject_ura)
    kind = projection.kind
    if kind in {"agent", "ability", "device", "resource"}:
        return subject_ura
    if kind in {"user", "hub"}:
        return identity.descriptor_bound_resource_subject_ura(
            subject_ura,
            f"invoke/{ability_name}",
        )
    raise _invalid_access_control(f"subject_ura kind {kind!r} is not descriptor-bound")


def _output_mapping(output: Any) -> Mapping[str, object]:
    if not isinstance(output, Mapping):
        raise _invalid_access_control("access-control output_json must be an object")
    return dict(output)


def _validate_carrier(base: AccessControlCarrierBase) -> None:
    if (
        not base.caller_ura
        or not base.callee_ura
        or not base.subject_ura
        or not base.descriptor_version
        or not base.nonce_base64
        or base.causal_context is None
    ):
        raise _invalid_access_control("complete access-control invocation carrier is required")


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
