"""Typed authority metadata facade for Invocation admission."""

from __future__ import annotations

import base64
import binascii
import json
from dataclasses import dataclass, field
from typing import Mapping, Protocol

from .errors import ErrorCode, RetryHint, SDKError

DELEGATION_METADATA_KEY = "x-easynet-delegation"
SESSION_AUTHORITY_METADATA_KEY = "x-easynet-session-authority"


@dataclass(frozen=True)
class AuthoritySigningMaterial:
    """Canonical authority material prepared by the runtime core."""

    profile: str
    kind: str
    algorithm: str
    metadata_key: str
    canonical_bytes_base64: str
    canonical_hash_hex: str
    signed_fields: tuple[str, ...]
    payload: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str, *, kind: str, metadata_key: str) -> "AuthoritySigningMaterial":
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        try:
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_authority(f"decode authority signing material: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_authority("authority signing material must be an object")
        material = cls(
            profile=_required_string(decoded.get("profile"), "profile"),
            kind=_required_string(decoded.get("kind"), "kind"),
            algorithm=_required_string(decoded.get("algorithm"), "algorithm"),
            metadata_key=_required_string(decoded.get("metadata_key"), "metadata_key"),
            canonical_bytes_base64=_required_string(
                decoded.get("canonical_bytes_base64"), "canonical_bytes_base64"
            ),
            canonical_hash_hex=_required_string(decoded.get("canonical_hash_hex"), "canonical_hash_hex"),
            signed_fields=_required_string_tuple(
                decoded.get("signed_fields"), "signed_fields", "authority"
            ),
            payload=_required_mapping(decoded.get("payload"), "payload"),
        )
        if material.profile != "authority" or material.kind != kind or material.metadata_key != metadata_key:
            raise _invalid_authority("authority signing material identity mismatch")
        _decode_base64(material.canonical_bytes_base64, "canonical_bytes_base64")
        return material


@dataclass(frozen=True)
class AuthoritySignature:
    """Latest-only authority signature envelope."""

    signature_base64: str

    def to_json(self) -> bytes:
        if not isinstance(self.signature_base64, str) or not self.signature_base64.strip():
            raise _invalid_authority("authority signature_base64 is required")
        _decode_base64(self.signature_base64, "signature_base64")
        return json.dumps(
            {"signature_base64": self.signature_base64},
            separators=(",", ":"),
        ).encode("utf-8")


class AuthoritySignatureProvider(Protocol):
    """Signs canonical authority material outside the C ABI."""

    def sign_authority(self, material: AuthoritySigningMaterial) -> AuthoritySignature:
        ...


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


@dataclass(frozen=True)
class DelegationRequest:
    """Typed request for delegated-authority metadata minting."""

    issuer_ura: str
    subject_ura: str
    caller_ura: str
    audience: str
    scopes: tuple[str, ...]
    issued_at_ms: int
    expires_at_ms: int
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json(self) -> bytes:
        _validate_delegation_request(self)
        return json.dumps(
            {
                "issuer_ura": self.issuer_ura,
                "subject_ura": self.subject_ura,
                "caller_ura": self.caller_ura,
                "audience": self.audience,
                "scopes": list(self.scopes),
                "issued_at_ms": self.issued_at_ms,
                "expires_at_ms": self.expires_at_ms,
                "metadata": dict(self.metadata),
            },
            separators=(",", ":"),
        ).encode("utf-8")


@dataclass(frozen=True)
class SessionAuthorityRequest:
    """Typed request for session-authority metadata minting."""

    backend_ura: str
    user_ura: str
    session_id: str
    scopes: tuple[str, ...]
    audiences: tuple[str, ...]
    issued_at_ms: int
    expires_at_ms: int
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json(self) -> bytes:
        _validate_session_authority_request(self)
        return json.dumps(
            {
                "backend_ura": self.backend_ura,
                "user_ura": self.user_ura,
                "session_id": self.session_id,
                "scopes": list(self.scopes),
                "audiences": list(self.audiences),
                "issued_at_ms": self.issued_at_ms,
                "expires_at_ms": self.expires_at_ms,
                "metadata": dict(self.metadata),
            },
            separators=(",", ":"),
        ).encode("utf-8")


class AuthorityTransport(Protocol):
    """Provider boundary for authority metadata minting."""

    def mint_delegation_proof(self, request_json: bytes) -> bytes:
        ...

    def mint_session_authority(self, request_json: bytes) -> bytes:
        ...


class AuthorityClient:
    """Typed authority metadata minting facade."""

    def __init__(self, transport: AuthorityTransport):
        if transport is None:
            raise _invalid_authority("authority transport is required")
        self._transport = transport
        self._closed = False

    def mint_delegation_proof(self, request: DelegationRequest) -> DelegationProof:
        self._require_open()
        if not isinstance(request, DelegationRequest):
            raise _invalid_authority("delegation request is required")
        raw = self._transport.mint_delegation_proof(request.to_json())
        value = _authority_metadata_projection(raw, DELEGATION_METADATA_KEY, "delegation")
        return DelegationProof.from_metadata(value)

    def mint_session_authority(
        self, request: SessionAuthorityRequest
    ) -> SessionAuthority:
        self._require_open()
        if not isinstance(request, SessionAuthorityRequest):
            raise _invalid_authority("session authority request is required")
        raw = self._transport.mint_session_authority(request.to_json())
        value = _authority_metadata_projection(
            raw, SESSION_AUTHORITY_METADATA_KEY, "session authority"
        )
        return SessionAuthority.from_metadata(value)

    def close(self) -> None:
        if self._closed:
            return
        close = getattr(self._transport, "close", None)
        self._closed = True
        if callable(close):
            close()

    def _require_open(self) -> None:
        if self._closed:
            raise _invalid_authority("authority client is closed")


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
        wire_bytes = _decode_base64(value.strip(), f"{label} metadata")
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
        signature = _decode_base64(signature_value, f"{label} metadata signature")
    except binascii.Error as exc:
        raise _invalid_authority(
            f"{label} metadata signature base64 decode failed: {exc}", exc
        ) from exc
    if not signature:
        raise _invalid_authority(f"{label} metadata signature is required")
    return payload, signature


def _authority_metadata_projection(raw: bytes | str, metadata_key: str, label: str) -> str:
    text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
    if not isinstance(text, str) or not text.strip():
        raise _invalid_authority(f"{label} metadata projection is required")
    stripped = text.strip()
    if stripped.startswith("{"):
        try:
            decoded = json.loads(stripped)
        except Exception as exc:
            raise _invalid_authority(
                f"decode {label} metadata projection: {exc}", exc
            ) from exc
        if not isinstance(decoded, dict):
            raise _invalid_authority(f"{label} metadata projection must be an object")
        for key in ("metadata_value", "value"):
            value = decoded.get(key)
            if isinstance(value, str) and value.strip():
                return value.strip()
        metadata = decoded.get("metadata")
        if isinstance(metadata, dict):
            return _authority_metadata_value(metadata, metadata_key)
        raise _invalid_authority(f"{label} metadata projection missing metadata_value")
    if stripped.startswith('"'):
        try:
            value = json.loads(stripped)
        except Exception as exc:
            raise _invalid_authority(
                f"decode {label} metadata value: {exc}", exc
            ) from exc
        if not isinstance(value, str):
            raise _invalid_authority(f"{label} metadata value must be a string")
        return value.strip()
    return stripped


def _required_payload_string(payload: Mapping[str, object], field_name: str, label: str) -> str:
    value = payload.get(field_name)
    return _required_string(value, f"{label} authority {field_name}")


def _required_string(value: object, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _invalid_authority(f"{label} is required")
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


def _required_mapping(value: object, label: str) -> Mapping[str, object]:
    if not isinstance(value, dict):
        raise _invalid_authority(f"authority signing material {label} is required")
    return value


def _decode_base64(value: str, label: str) -> bytes:
    try:
        decoded = base64.b64decode(value, validate=True)
    except binascii.Error as exc:
        raise _invalid_authority(f"{label} base64 decode failed: {exc}", exc) from exc
    if not decoded:
        raise _invalid_authority(f"{label} is required")
    return decoded


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


def _validate_delegation_request(request: DelegationRequest) -> None:
    _validate_delegation(
        DelegationProof(
            issuer_ura=request.issuer_ura,
            subject_ura=request.subject_ura,
            caller_ura=request.caller_ura,
            audience=request.audience,
            scopes=tuple(request.scopes),
            issued_at_ms=request.issued_at_ms,
            expires_at_ms=request.expires_at_ms,
            signature=b"shape-only",
        )
    )
    _reject_private_key_metadata(request.metadata)


def _validate_session_authority_request(request: SessionAuthorityRequest) -> None:
    _validate_session_authority(
        SessionAuthority(
            backend_ura=request.backend_ura,
            user_ura=request.user_ura,
            session_id=request.session_id,
            scopes=tuple(request.scopes),
            audiences=tuple(request.audiences),
            issued_at_ms=request.issued_at_ms,
            expires_at_ms=request.expires_at_ms,
            signature=b"shape-only",
        )
    )
    _reject_private_key_metadata(request.metadata)


def _reject_private_key_metadata(metadata: Mapping[str, object]) -> None:
    for key in metadata.keys():
        if key.strip().lower() in {
            "private_key",
            "private_key_seed",
            "private_key_seed_base64",
            "private_key_hex",
            "signing_key",
            "ed25519_seed",
        }:
            raise _invalid_authority(
                "private key material must not be supplied to authority facade"
            )


def _invalid_authority(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="build",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )
