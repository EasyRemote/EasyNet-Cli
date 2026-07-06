"""Typed authority metadata facade for Invocation admission."""

from __future__ import annotations

import base64
import binascii
import json
from dataclasses import dataclass, field
from typing import Mapping

from .errors import ErrorCode, RetryHint, SDKError

DELEGATION_METADATA_KEY = "x-easynet-delegation"
SESSION_AUTHORITY_METADATA_KEY = "x-easynet-session-authority"


@dataclass(frozen=True)
class AuthorityMetadata:
    """Mutually-exclusive authority metadata envelope for one Invocation."""

    kind: str
    key: str
    value: str

    def to_metadata_dict(self) -> dict[str, object]:
        if not self.key.strip() or not self.value.strip():
            return {}
        return {self.key: self.value}

    def merge_into(self, metadata: Mapping[str, object]) -> dict[str, object]:
        if not self.key.strip() or not self.value.strip():
            raise _invalid_authority("authority metadata is empty")
        merged = dict(metadata)
        merged[self.key] = self.value
        validate_authority_metadata(merged)
        return merged


@dataclass(frozen=True)
class DelegationProof:
    """Typed projection of daemon/Axon delegated-authority metadata."""

    issuer_ura: str
    subject_ura: str
    caller_ura: str
    audience: str
    scopes: tuple[str, ...]
    issued_at_ms: int
    expires_at_ms: int
    signature: bytes
    metadata_value: str = field(default="", repr=False)

    @classmethod
    def from_metadata(cls, value: str) -> "DelegationProof":
        payload, signature = _decode_authority_metadata(value, "delegation")
        proof = cls(
            issuer_ura=_required_payload_string(payload, "issuer_ura", "delegation"),
            subject_ura=_required_payload_string(payload, "subject_ura", "delegation"),
            caller_ura=_required_payload_string(payload, "caller_ura", "delegation"),
            audience=_required_payload_string(payload, "audience", "delegation"),
            scopes=_required_string_tuple(payload.get("scopes"), "scopes", "delegation"),
            issued_at_ms=_required_payload_int(payload, "issued_at_ms", "delegation"),
            expires_at_ms=_required_payload_int(payload, "expires_at_ms", "delegation"),
            signature=signature,
            metadata_value=value.strip(),
        )
        _validate_delegation(proof)
        return proof

    def metadata(self) -> AuthorityMetadata:
        _validate_delegation(self)
        if not self.metadata_value.strip():
            raise _invalid_authority("delegation metadata value is required")
        return AuthorityMetadata(
            kind="delegation",
            key=DELEGATION_METADATA_KEY,
            value=self.metadata_value,
        )


@dataclass(frozen=True)
class SessionAuthority:
    """Typed projection of daemon/Axon session-authority metadata."""

    backend_ura: str
    user_ura: str
    session_id: str
    scopes: tuple[str, ...]
    audiences: tuple[str, ...]
    issued_at_ms: int
    expires_at_ms: int
    signature: bytes
    metadata_value: str = field(default="", repr=False)

    @classmethod
    def from_metadata(cls, value: str) -> "SessionAuthority":
        payload, signature = _decode_authority_metadata(value, "session authority")
        authority = cls(
            backend_ura=_required_payload_string(payload, "backend_ura", "session authority"),
            user_ura=_required_payload_string(payload, "user_ura", "session authority"),
            session_id=_required_payload_string(payload, "session_id", "session authority"),
            scopes=_required_string_tuple(payload.get("scopes"), "scopes", "session authority"),
            audiences=_required_string_tuple(
                payload.get("audiences"), "audiences", "session authority"
            ),
            issued_at_ms=_required_payload_int(payload, "issued_at_ms", "session authority"),
            expires_at_ms=_required_payload_int(payload, "expires_at_ms", "session authority"),
            signature=signature,
            metadata_value=value.strip(),
        )
        _validate_session_authority(authority)
        return authority

    def metadata(self) -> AuthorityMetadata:
        _validate_session_authority(self)
        if not self.metadata_value.strip():
            raise _invalid_authority("session authority metadata value is required")
        return AuthorityMetadata(
            kind="session_authority",
            key=SESSION_AUTHORITY_METADATA_KEY,
            value=self.metadata_value,
        )


def validate_authority_metadata(metadata: Mapping[str, object]) -> None:
    delegation = _authority_metadata_value(metadata, DELEGATION_METADATA_KEY)
    session = _authority_metadata_value(metadata, SESSION_AUTHORITY_METADATA_KEY)
    if delegation and session:
        raise _invalid_authority("invocation authority metadata is ambiguous")


def _authority_metadata_value(metadata: Mapping[str, object], key: str) -> str:
    raw = metadata.get(key)
    if raw is None:
        return ""
    if not isinstance(raw, str):
        raise _invalid_authority(f"{key} must be a string metadata value")
    return raw.strip()


def _decode_authority_metadata(value: str, label: str) -> tuple[Mapping[str, object], bytes]:
    if not isinstance(value, str) or not value.strip():
        raise _invalid_authority(f"{label} metadata value is required")
    try:
        wire_bytes = base64.b64decode(value.strip(), validate=True)
    except binascii.Error as exc:
        raise _invalid_authority(f"{label} metadata base64 decode failed: {exc}", exc) from exc
    try:
        wire = json.loads(wire_bytes.decode("utf-8"))
    except Exception as exc:
        raise _invalid_authority(f"{label} metadata JSON parse failed: {exc}", exc) from exc
    if not isinstance(wire, dict):
        raise _invalid_authority(f"{label} metadata JSON must be an object")
    payload = wire.get("payload")
    if not isinstance(payload, dict):
        raise _invalid_authority(f"{label} metadata payload is required")
    signature_value = wire.get("signature")
    if not isinstance(signature_value, str) or not signature_value.strip():
        raise _invalid_authority(f"{label} metadata signature is required")
    try:
        signature = base64.b64decode(signature_value, validate=True)
    except binascii.Error as exc:
        raise _invalid_authority(
            f"{label} metadata signature base64 decode failed: {exc}", exc
        ) from exc
    if not signature:
        raise _invalid_authority(f"{label} metadata signature is required")
    return payload, signature


def _required_payload_string(payload: Mapping[str, object], field_name: str, label: str) -> str:
    value = payload.get(field_name)
    if not isinstance(value, str) or not value.strip():
        raise _invalid_authority(f"{label} authority {field_name} is required")
    return value


def _required_payload_int(payload: Mapping[str, object], field_name: str, label: str) -> int:
    value = payload.get(field_name)
    if isinstance(value, bool) or not isinstance(value, int):
        raise _invalid_authority(f"{label} authority {field_name} is required")
    return value


def _required_string_tuple(value: object, field_name: str, label: str) -> tuple[str, ...]:
    if not isinstance(value, list) or len(value) == 0:
        raise _invalid_authority(f"{label} authority {field_name} are required")
    result: list[str] = []
    for item in value:
        if not isinstance(item, str) or not item.strip():
            raise _invalid_authority(f"{label} authority {field_name} must be strings")
        result.append(item)
    return tuple(result)


def _validate_delegation(proof: DelegationProof) -> None:
    if not (
        proof.issuer_ura.strip()
        and proof.subject_ura.strip()
        and proof.caller_ura.strip()
        and proof.audience.strip()
    ):
        raise _invalid_authority("delegation authority must bind issuer, subject, caller, and audience")
    if not proof.scopes:
        raise _invalid_authority("delegation authority scopes are required")
    if proof.expires_at_ms <= proof.issued_at_ms:
        raise _invalid_authority("delegation authority expires_at_ms must be greater than issued_at_ms")
    if not proof.signature:
        raise _invalid_authority("delegation authority signature is required")


def _validate_session_authority(authority: SessionAuthority) -> None:
    if not (
        authority.backend_ura.strip()
        and authority.user_ura.strip()
        and authority.session_id.strip()
    ):
        raise _invalid_authority("session authority must bind backend, user, and session_id")
    if not authority.scopes or not authority.audiences:
        raise _invalid_authority("session authority scopes and audiences are required")
    if authority.expires_at_ms <= authority.issued_at_ms:
        raise _invalid_authority(
            "session authority expires_at_ms must be greater than issued_at_ms"
        )
    if not authority.signature:
        raise _invalid_authority("session authority signature is required")


def _invalid_authority(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="build",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )
