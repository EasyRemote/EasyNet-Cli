"""Local runtime authority binding over explicit Invocation drafts.

The provider binds User-owned subjects with delegation and exact local
Device-owned Resources with session authority. Key custody, rotation, and the
final Device/User ownership decision remain behind the daemon key service and
admission boundary; consumers never enumerate key inventory.
"""

from __future__ import annotations

import hashlib
import time
from collections.abc import Callable
from dataclasses import replace
from typing import Protocol

from .authority import (
    DELEGATION_METADATA_KEY,
    SESSION_AUTHORITY_METADATA_KEY,
    CanonicalSigner,
    DelegationRequest,
    SessionAuthorityRequest,
    new_canonical_authority_client,
)
from .axon_addressing import AddressingClient, AddressingProjection
from .errors import ErrorCode, RetryHint, SDKError
from .invocation import InvocationDraft
from .managed_signing import ManagedSigningClient
from .runtime_signer import USER_RUNTIME_SIGNING_PURPOSE


class DraftAuthorityProvider(Protocol):
    """Bind authority metadata to one already-finalized Invocation draft."""

    def bind(self, draft: InvocationDraft) -> InvocationDraft: ...


class LocalRuntimeAuthorityProvider:
    """Daemon-backed authority policy for local SDK attachments."""

    def __init__(
        self,
        addressing: AddressingClient,
        *,
        key_service_path: str = "",
        signer_loader: Callable[[str], CanonicalSigner] | None = None,
        clock_ms: Callable[[], int] | None = None,
        authority_ttl_ms: int = 5 * 60 * 1000,
    ) -> None:
        if addressing is None:
            raise _authority_error(
                ErrorCode.PROVIDER_UNAVAILABLE,
                "Addressing provider is required",
            )
        if authority_ttl_ms <= 0:
            raise _authority_error(
                ErrorCode.INVALID_ARGUMENT,
                "authority_ttl_ms must be positive",
            )
        self._addressing = addressing
        self._signer_loader = signer_loader or (
            lambda owner_ura: ManagedSigningClient(
                key_service_path
            ).active_signer_for_subject(
                owner_ura,
                purpose=USER_RUNTIME_SIGNING_PURPOSE,
            )
        )
        self._clock_ms = clock_ms or (lambda: int(time.time() * 1000))
        self._authority_ttl_ms = authority_ttl_ms

    def bind(self, draft: InvocationDraft) -> InvocationDraft:
        """Bind exact authority for one User caller and protected subject."""

        if not isinstance(draft, InvocationDraft):
            raise _authority_error(
                ErrorCode.INVALID_INVOCATION,
                "InvocationDraft is required",
            )
        if _has_authority(draft):
            return draft
        caller = self._addressing.parse_ura(draft.caller_ura)
        if caller.kind != "user":
            return draft
        caller_user_id = _component(caller, "user_id")
        subject = self._addressing.parse_ura(draft.subject_ura)
        device_id = _device_resource_id(subject)
        if device_id:
            _require_local_device_resource_geometry(
                self._addressing,
                caller,
                draft,
                device_id,
            )
            return self._bind_device_resource_session(
                draft,
                caller_user_id=caller_user_id,
            )

        subject_user_id = _subject_user_id(self._addressing, subject)
        if not subject_user_id or draft.subject_ura == draft.caller_ura:
            return draft
        if subject_user_id != caller_user_id:
            raise _authority_error(
                ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
                "caller cannot authorize a subject owned by another User",
            )
        scope = _descriptor_scope(self._addressing, draft.descriptor_ref)
        now_ms = self._clock_ms()
        signer = self._signer_loader(draft.caller_ura)
        authority_client = new_canonical_authority_client(signer)
        try:
            authority = authority_client.mint_delegation_proof(
                DelegationRequest(
                    issuer_ura=draft.caller_ura,
                    subject_ura=draft.subject_ura,
                    caller_ura=draft.caller_ura,
                    audience=draft.callee_ura,
                    scopes=(scope,),
                    issued_at_ms=now_ms,
                    expires_at_ms=now_ms + self._authority_ttl_ms,
                )
            )
        finally:
            authority_client.close()
        return replace(
            draft,
            metadata=authority.metadata().merge_into(draft.metadata),
        )

    def _bind_device_resource_session(
        self,
        draft: InvocationDraft,
        *,
        caller_user_id: str,
    ) -> InvocationDraft:
        scope, action = _descriptor_contract(self._addressing, draft.descriptor_ref)
        now_ms = self._clock_ms()
        signer = self._signer_loader(draft.caller_ura)
        authority_client = new_canonical_authority_client(signer)
        try:
            authority = authority_client.mint_session_authority(
                SessionAuthorityRequest(
                    issuer_ura=draft.caller_ura,
                    session_id=_invocation_session_id(draft.nonce_base64),
                    session_owner_user_id=caller_user_id,
                    creator_principal_id=draft.caller_ura,
                    callee_ura=draft.callee_ura,
                    subject_ura=draft.subject_ura,
                    audience=draft.callee_ura,
                    scopes=(scope,),
                    allowed_actions=(action,),
                    allowed_followup_abilities=(scope,),
                    issued_at_ms=now_ms,
                    expires_at_ms=now_ms + self._authority_ttl_ms,
                    session_owner_ura=draft.caller_ura,
                    creator_principal_ura=draft.caller_ura,
                )
            )
        finally:
            authority_client.close()
        return replace(
            draft,
            metadata=authority.metadata().merge_into(draft.metadata),
        )


def _has_authority(draft: InvocationDraft) -> bool:
    return any(
        draft.metadata.get(key)
        for key in (DELEGATION_METADATA_KEY, SESSION_AUTHORITY_METADATA_KEY)
    )


def _subject_user_id(
    addressing: AddressingClient,
    subject: AddressingProjection,
) -> str:
    if subject.kind in {"user", "agent"}:
        return _component(subject, "user_id")
    if subject.kind == "ability":
        owner_ura = _component(subject, "owner_ura")
        if not owner_ura:
            return ""
        return _subject_user_id(addressing, addressing.parse_ura(owner_ura))
    if subject.kind != "resource":
        return ""
    owner_id = _component(subject, "owner_id")
    if owner_id.startswith("user."):
        return owner_id.removeprefix("user.").strip()
    if owner_id.startswith("agent."):
        owner = owner_id.removeprefix("agent.").split(".", 1)
        return owner[0].strip() if len(owner) == 2 else ""
    return ""


def _descriptor_scope(addressing: AddressingClient, descriptor_ref: str) -> str:
    return _descriptor_contract(addressing, descriptor_ref)[0]


def _descriptor_contract(
    addressing: AddressingClient, descriptor_ref: str
) -> tuple[str, str]:
    descriptor = addressing.project_descriptor_ref(descriptor_ref)
    ability = addressing.project_ability_ura(descriptor.ability_ura)
    scope = ability.public_name.strip() or descriptor.ability_ura.strip()
    if not scope or scope == "*":
        raise _authority_error(
            ErrorCode.INVALID_INVOCATION,
            "descriptor did not resolve to an exact authority scope",
        )
    action = descriptor.action.strip()
    if not action or action == "*":
        raise _authority_error(
            ErrorCode.INVALID_INVOCATION,
            "descriptor did not resolve to an exact authority action",
        )
    return scope, action


def _device_resource_id(subject: AddressingProjection) -> str:
    if subject.kind != "resource":
        return ""
    owner_id = _component(subject, "owner_id")
    if not owner_id.startswith("device."):
        return ""
    return owner_id.removeprefix("device.").strip()


def _require_local_device_resource_geometry(
    addressing: AddressingClient,
    caller: AddressingProjection,
    draft: InvocationDraft,
    device_id: str,
) -> None:
    callee = addressing.parse_ura(draft.callee_ura)
    callee_device_id = _component(callee, "device_id")
    if (
        not device_id
        or caller.realm != callee.realm
        or callee.kind != "agent"
        or callee_device_id != device_id
    ):
        raise _authority_error(
            ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
            "Device Resource and device-sponsored callee must identify the same local Device",
        )


def _invocation_session_id(nonce_base64: str) -> str:
    digest = hashlib.sha256(nonce_base64.encode("ascii")).hexdigest()
    return f"invoke-{digest}"


def _component(projection: AddressingProjection, key: str) -> str:
    value = projection.components.get(key)
    return value.strip() if isinstance(value, str) else ""


def _authority_error(code: ErrorCode, message: str) -> SDKError:
    return SDKError(
        code=code,
        stage="runtime_authority",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )
