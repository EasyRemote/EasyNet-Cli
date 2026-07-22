"""Typed authority metadata facade for Invocation admission."""

from __future__ import annotations

import base64
import binascii
import json
from dataclasses import dataclass, field
from typing import Any, Mapping, Protocol, cast

from .axon_addressing import parse_ura, user_ura
from .errors import ErrorCode, RetryHint, SDKError
from ._identity_guards import contains_all_zero_principal

DELEGATION_METADATA_KEY = "x-runtime-delegation"
SESSION_AUTHORITY_METADATA_KEY = "x-runtime-session-authority"


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
    def from_json(
        cls, raw: bytes | str, *, kind: str, metadata_key: str
    ) -> "AuthoritySigningMaterial":
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        try:
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_authority(
                f"decode authority signing material: {exc}", exc
            ) from exc
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
            canonical_hash_hex=_required_string(
                decoded.get("canonical_hash_hex"), "canonical_hash_hex"
            ),
            signed_fields=_required_string_tuple(
                decoded.get("signed_fields"), "signed_fields", "authority"
            ),
            payload=_required_mapping(decoded.get("payload"), "payload"),
        )
        if (
            material.profile != "authority"
            or material.kind != kind
            or material.metadata_key != metadata_key
        ):
            raise _invalid_authority("authority signing material identity mismatch")
        _decode_base64(material.canonical_bytes_base64, "canonical_bytes_base64")
        return material


@dataclass(frozen=True)
class AuthoritySignature:
    """Latest-only authority signature envelope."""

    signature_base64: str

    def to_json(self) -> bytes:
        if (
            not isinstance(self.signature_base64, str)
            or not self.signature_base64.strip()
        ):
            raise _invalid_authority("authority signature_base64 is required")
        _decode_base64(self.signature_base64, "signature_base64")
        return json.dumps(
            {"signature_base64": self.signature_base64},
            separators=(",", ":"),
        ).encode("utf-8")


class AuthoritySignatureProvider(Protocol):
    """Signs canonical authority material outside the C ABI."""

    def sign_authority(
        self, material: AuthoritySigningMaterial
    ) -> AuthoritySignature: ...


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
    """Typed projection of runtime delegated-authority metadata."""

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
            scopes=_required_string_tuple(
                payload.get("scopes"), "scopes", "delegation"
            ),
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

    def matches_scope(self, ability: str) -> bool:
        """Return whether this proof admits the canonical ability selector."""

        return any(_match_scope_pattern(scope, ability) for scope in self.scopes)

    def matches_audience(self, callee_ura: str) -> bool:
        """Return whether this proof admits the canonical callee."""

        return _audience_admits(self.audience, callee_ura)


@dataclass(frozen=True)
class SessionAuthority:
    """Typed projection of runtime session-authority metadata."""

    issuer_ura: str
    session_id: str
    session_owner_user_id: str
    creator_principal_id: str
    callee_ura: str
    subject_ura: str
    audience: str
    scopes: tuple[str, ...]
    allowed_actions: tuple[str, ...]
    allowed_followup_abilities: tuple[str, ...]
    issued_at_ms: int
    expires_at_ms: int
    signature: bytes
    session_owner_ura: str = ""
    creator_principal_ura: str = ""
    metadata_value: str = field(default="", repr=False)

    @classmethod
    def from_metadata(cls, value: str) -> "SessionAuthority":
        payload, signature = _decode_authority_metadata(value, "session authority")
        authority = cls(
            issuer_ura=_required_payload_string(
                payload, "issuer_ura", "session authority"
            ),
            session_id=_required_payload_string(
                payload, "session_id", "session authority"
            ),
            session_owner_user_id=_required_payload_string(
                payload, "session_owner_user_id", "session authority"
            ),
            creator_principal_id=_required_payload_string(
                payload, "creator_principal_id", "session authority"
            ),
            callee_ura=_required_payload_string(
                payload, "callee_ura", "session authority"
            ),
            subject_ura=_required_payload_string(
                payload, "subject_ura", "session authority"
            ),
            audience=_required_payload_string(payload, "audience", "session authority"),
            scopes=_required_string_tuple(
                payload.get("scopes"), "scopes", "session authority"
            ),
            allowed_actions=_required_string_tuple(
                payload.get("allowed_actions"), "allowed_actions", "session authority"
            ),
            allowed_followup_abilities=_required_string_tuple(
                payload.get("allowed_followup_abilities"),
                "allowed_followup_abilities",
                "session authority",
            ),
            issued_at_ms=_required_payload_int(
                payload, "issued_at_ms", "session authority"
            ),
            expires_at_ms=_required_payload_int(
                payload, "expires_at_ms", "session authority"
            ),
            signature=signature,
            session_owner_ura=_session_owner_ura_from_payload(payload),
            creator_principal_ura=_canonical_ura_or_empty(
                _required_payload_string(
                    payload, "creator_principal_id", "session authority"
                )
            ),
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

    def matches_scope(self, ability: str) -> bool:
        """Return whether this authority admits the canonical ability selector."""

        return any(_match_scope_pattern(scope, ability) for scope in self.scopes)

    def matches_audience(self, callee_ura: str) -> bool:
        """Return whether this authority admits the canonical callee."""

        return _audience_admits(self.audience, callee_ura)


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

    issuer_ura: str
    session_id: str
    session_owner_user_id: str
    creator_principal_id: str
    callee_ura: str
    subject_ura: str
    audience: str
    scopes: tuple[str, ...]
    allowed_actions: tuple[str, ...]
    allowed_followup_abilities: tuple[str, ...]
    issued_at_ms: int
    expires_at_ms: int
    session_owner_ura: str = ""
    creator_principal_ura: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json(self) -> bytes:
        wire = _session_authority_request_wire(self)
        return json.dumps(
            wire,
            separators=(",", ":"),
        ).encode("utf-8")


class AuthorityTransport(Protocol):
    """Provider boundary for authority metadata minting."""

    def mint_delegation_proof(self, request_json: bytes) -> bytes: ...

    def mint_session_authority(self, request_json: bytes) -> bytes: ...


class CanonicalSigner(Protocol):
    """Opaque signer for SDK-owned canonical authority payloads."""

    def sign_canonical(self, canonical_bytes: bytes) -> bytes: ...


class CanonicalAuthorityTransport:
    """Authority provider backed by one opaque canonical signer.

    The transport owns authority DTO construction and raw metadata encoding;
    the signer owns key custody. It has no product, daemon, or identity policy.
    """

    def __init__(self, signer: CanonicalSigner):
        if signer is None or not callable(getattr(signer, "sign_canonical", None)):
            raise _invalid_authority("canonical authority signer is required")
        self._signer: CanonicalSigner | None = signer
        self._closed = False

    def mint_delegation_proof(self, request_json: bytes) -> bytes:
        self._require_open()
        request = _decode_delegation_request(request_json)
        payload = _delegation_payload(request)
        signature = self._sign(payload)
        return _authority_projection_bytes(DELEGATION_METADATA_KEY, payload, signature)

    def mint_session_authority(self, request_json: bytes) -> bytes:
        self._require_open()
        request = _decode_session_authority_request(request_json)
        payload = _session_authority_payload(request)
        signature = self._sign(payload)
        return _authority_projection_bytes(
            SESSION_AUTHORITY_METADATA_KEY, payload, signature
        )

    def close(self) -> None:
        self._closed = True
        self._signer = None

    def _require_open(self) -> None:
        if self._closed or self._signer is None:
            raise _invalid_authority("canonical authority transport is closed")

    def _sign(self, payload: bytes) -> bytes:
        try:
            signer = self._signer
            if signer is None:
                raise _invalid_authority("canonical authority transport is closed")
            signature = signer.sign_canonical(payload)
        except SDKError:
            raise
        except Exception as exc:
            raise _invalid_authority("canonical authority signing failed", exc) from exc
        if not isinstance(signature, bytes) or len(signature) != 64:
            raise _invalid_authority(
                "canonical authority signer must return a 64-byte signature"
            )
        return signature


def new_canonical_authority_client(signer: CanonicalSigner) -> "AuthorityClient":
    """Create an AuthorityClient over the generic canonical provider."""

    return AuthorityClient(CanonicalAuthorityTransport(signer))


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
        value = _authority_metadata_projection(
            raw, DELEGATION_METADATA_KEY, "delegation"
        )
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


def _match_scope_pattern(pattern: str, ability: str) -> bool:
    if pattern == "*":
        return True
    if len(pattern) >= 2 and pattern.endswith("*"):
        return ability.startswith(pattern[:-1])
    return pattern == ability


def _audience_admits(audience: str, callee_ura: str) -> bool:
    if audience == "*" or audience == callee_ura:
        return True
    return bool(audience) and audience.endswith("/") and callee_ura.startswith(audience)


def _decode_authority_metadata(
    value: str, label: str
) -> tuple[Mapping[str, object], bytes]:
    if not isinstance(value, str) or not value.strip():
        raise _invalid_authority(f"{label} metadata value is required")
    try:
        wire_bytes = _decode_base64(value.strip(), f"{label} metadata")
    except binascii.Error as exc:
        raise _invalid_authority(
            f"{label} metadata base64 decode failed: {exc}", exc
        ) from exc
    try:
        wire = json.loads(wire_bytes.decode("utf-8"))
    except Exception as exc:
        raise _invalid_authority(
            f"{label} metadata JSON parse failed: {exc}", exc
        ) from exc
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


def _authority_metadata_projection(
    raw: bytes | str, metadata_key: str, label: str
) -> str:
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


def _decode_delegation_request(raw: bytes) -> DelegationRequest:
    decoded = _decode_authority_request(raw, "delegation")
    try:
        request = DelegationRequest(
            issuer_ura=cast(str, decoded["issuer_ura"]),
            subject_ura=cast(str, decoded["subject_ura"]),
            caller_ura=cast(str, decoded["caller_ura"]),
            audience=cast(str, decoded["audience"]),
            scopes=tuple(cast(tuple[str, ...], decoded["scopes"])),
            issued_at_ms=cast(int, decoded["issued_at_ms"]),
            expires_at_ms=cast(int, decoded["expires_at_ms"]),
            metadata=cast(Mapping[str, object], decoded.get("metadata", {})),
        )
        request.to_json()
        return request
    except (KeyError, TypeError, ValueError) as exc:
        raise _invalid_authority("decode delegation request", exc) from exc


def _decode_session_authority_request(raw: bytes) -> SessionAuthorityRequest:
    decoded = _decode_authority_request(raw, "session authority")
    try:
        request = SessionAuthorityRequest(
            issuer_ura=cast(str, decoded["issuer_ura"]),
            session_id=cast(str, decoded["session_id"]),
            session_owner_user_id=str(decoded.get("session_owner_user_id", "")),
            creator_principal_id=str(decoded.get("creator_principal_id", "")),
            callee_ura=cast(str, decoded["callee_ura"]),
            subject_ura=cast(str, decoded["subject_ura"]),
            audience=cast(str, decoded["audience"]),
            scopes=tuple(cast(tuple[str, ...], decoded["scopes"])),
            allowed_actions=tuple(cast(tuple[str, ...], decoded["allowed_actions"])),
            allowed_followup_abilities=tuple(
                cast(tuple[str, ...], decoded["allowed_followup_abilities"])
            ),
            issued_at_ms=cast(int, decoded["issued_at_ms"]),
            expires_at_ms=cast(int, decoded["expires_at_ms"]),
            session_owner_ura=str(decoded.get("session_owner_ura", "")),
            creator_principal_ura=str(decoded.get("creator_principal_ura", "")),
            metadata=cast(Mapping[str, object], decoded.get("metadata", {})),
        )
        request.to_json()
        return request
    except (KeyError, TypeError, ValueError) as exc:
        raise _invalid_authority("decode session authority request", exc) from exc


def _decode_authority_request(raw: bytes, label: str) -> Mapping[str, object]:
    if not isinstance(raw, bytes) or not raw:
        raise _invalid_authority(f"{label} authority request JSON is required")
    try:
        decoded = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise _invalid_authority(f"decode {label} authority request", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_authority(f"{label} authority request must be an object")
    return decoded


def _delegation_payload(request: DelegationRequest) -> bytes:
    return _canonical_json(
        {
            "audience": request.audience,
            "caller_ura": request.caller_ura,
            "expires_at_ms": request.expires_at_ms,
            "issued_at_ms": request.issued_at_ms,
            "issuer_ura": request.issuer_ura,
            "scopes": list(request.scopes),
            "subject_ura": request.subject_ura,
        }
    )


def _session_authority_payload(request: SessionAuthorityRequest) -> bytes:
    wire = _session_authority_request_wire(request)
    return _canonical_json(
        {
            "allowed_actions": wire["allowed_actions"],
            "allowed_followup_abilities": wire["allowed_followup_abilities"],
            "audience": wire["audience"],
            "callee_ura": wire["callee_ura"],
            "creator_principal_id": wire["creator_principal_id"],
            "expires_at_ms": wire["expires_at_ms"],
            "issued_at_ms": wire["issued_at_ms"],
            "issuer_ura": wire["issuer_ura"],
            "scopes": wire["scopes"],
            "session_id": wire["session_id"],
            "session_owner_user_id": wire["session_owner_user_id"],
            "subject_ura": wire["subject_ura"],
        }
    )


def _canonical_json(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _authority_projection_bytes(key: str, payload: bytes, signature: bytes) -> bytes:
    raw = _canonical_json(
        {
            "payload": json.loads(payload.decode("utf-8")),
            "signature": base64.b64encode(signature).decode("ascii"),
        }
    )
    value = base64.b64encode(raw).decode("ascii")
    return _canonical_json({"metadata": {key: value}})


def _required_payload_string(
    payload: Mapping[str, object], field_name: str, label: str
) -> str:
    value = payload.get(field_name)
    return _required_string(value, f"{label} authority {field_name}")


def _required_string(value: object, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _invalid_authority(f"{label} is required")
    return value


def _required_payload_int(
    payload: Mapping[str, object], field_name: str, label: str
) -> int:
    value = payload.get(field_name)
    if isinstance(value, bool) or not isinstance(value, int):
        raise _invalid_authority(f"{label} authority {field_name} is required")
    return value


def _required_string_tuple(
    value: object, field_name: str, label: str
) -> tuple[str, ...]:
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


def _session_authority_request_wire(
    request: SessionAuthorityRequest,
) -> dict[str, object]:
    normalized = _normalized_session_authority_request(request)
    _validate_session_authority_request(normalized)
    return {
        "issuer_ura": normalized.issuer_ura,
        "session_id": normalized.session_id,
        "session_owner_user_id": normalized.session_owner_user_id,
        "creator_principal_id": normalized.creator_principal_id,
        "callee_ura": normalized.callee_ura,
        "subject_ura": normalized.subject_ura,
        "audience": normalized.audience,
        "scopes": list(normalized.scopes),
        "allowed_actions": list(normalized.allowed_actions),
        "allowed_followup_abilities": list(normalized.allowed_followup_abilities),
        "issued_at_ms": normalized.issued_at_ms,
        "expires_at_ms": normalized.expires_at_ms,
        "metadata": dict(normalized.metadata),
    }


def _normalized_session_authority_request(
    request: SessionAuthorityRequest,
) -> SessionAuthorityRequest:
    session_owner_user_id = request.session_owner_user_id.strip()
    session_owner_ura = request.session_owner_ura.strip()
    if not session_owner_ura:
        session_owner_ura = _session_owner_ura_from_subject(
            request.subject_ura, session_owner_user_id
        )
    if session_owner_ura:
        derived = _user_id_from_user_ura(session_owner_ura, "session_owner_ura")
        if session_owner_user_id and session_owner_user_id != derived:
            raise _invalid_authority(
                "session_owner_user_id must match session_owner_ura user id"
            )
        session_owner_user_id = derived

    creator_principal_id = request.creator_principal_id.strip()
    creator_principal_ura = request.creator_principal_ura.strip()
    if creator_principal_ura:
        _parse_required_ura(creator_principal_ura, "creator_principal_ura")
        if creator_principal_id and creator_principal_id != creator_principal_ura:
            raise _invalid_authority(
                "creator_principal_id must match creator_principal_ura"
            )
        creator_principal_id = creator_principal_ura
    elif creator_principal_id.startswith("easynet:///"):
        try:
            _parse_required_ura(creator_principal_id, "creator_principal_id")
            creator_principal_ura = creator_principal_id
        except SDKError:
            creator_principal_ura = ""

    return SessionAuthorityRequest(
        issuer_ura=request.issuer_ura,
        session_id=request.session_id,
        session_owner_user_id=session_owner_user_id,
        creator_principal_id=creator_principal_id,
        callee_ura=request.callee_ura,
        subject_ura=request.subject_ura,
        audience=request.audience,
        scopes=request.scopes,
        allowed_actions=request.allowed_actions,
        allowed_followup_abilities=request.allowed_followup_abilities,
        issued_at_ms=request.issued_at_ms,
        expires_at_ms=request.expires_at_ms,
        session_owner_ura=session_owner_ura,
        creator_principal_ura=creator_principal_ura,
        metadata=request.metadata,
    )


def _session_authority_with_canonical_principals(
    authority: SessionAuthority,
) -> SessionAuthority:
    request = _normalized_session_authority_request(
        SessionAuthorityRequest(
            issuer_ura=authority.issuer_ura,
            session_id=authority.session_id,
            session_owner_user_id=authority.session_owner_user_id,
            creator_principal_id=authority.creator_principal_id,
            callee_ura=authority.callee_ura,
            subject_ura=authority.subject_ura,
            audience=authority.audience,
            scopes=authority.scopes,
            allowed_actions=authority.allowed_actions,
            allowed_followup_abilities=authority.allowed_followup_abilities,
            issued_at_ms=authority.issued_at_ms,
            expires_at_ms=authority.expires_at_ms,
            session_owner_ura=authority.session_owner_ura,
            creator_principal_ura=authority.creator_principal_ura,
        )
    )
    return SessionAuthority(
        issuer_ura=authority.issuer_ura,
        session_id=authority.session_id,
        session_owner_user_id=request.session_owner_user_id,
        creator_principal_id=request.creator_principal_id,
        callee_ura=authority.callee_ura,
        subject_ura=authority.subject_ura,
        audience=authority.audience,
        scopes=authority.scopes,
        allowed_actions=authority.allowed_actions,
        allowed_followup_abilities=authority.allowed_followup_abilities,
        issued_at_ms=authority.issued_at_ms,
        expires_at_ms=authority.expires_at_ms,
        signature=authority.signature,
        session_owner_ura=request.session_owner_ura,
        creator_principal_ura=request.creator_principal_ura,
        metadata_value=authority.metadata_value,
    )


def _session_owner_ura_from_payload(payload: Mapping[str, object]) -> str:
    return _session_owner_ura_from_subject(
        str(payload.get("subject_ura", "")),
        str(payload.get("session_owner_user_id", "")),
    )


def _session_owner_ura_from_subject(subject_ura: str, owner_user_id: str) -> str:
    owner_user_id = owner_user_id.strip()
    if not owner_user_id:
        return ""
    try:
        projection = parse_ura(subject_ura.strip())
    except SDKError:
        return ""
    if (
        projection.kind == "user"
        and projection.components.get("user_id") == owner_user_id
    ):
        return projection.ura
    if (
        projection.kind == "resource"
        and projection.components.get("owner_id") == f"user.{owner_user_id}"
    ):
        return user_ura(projection.realm, owner_user_id)
    return ""


def _user_id_from_user_ura(value: str, field_name: str) -> str:
    projection = _parse_required_ura(value, field_name)
    if projection.kind != "user":
        raise _invalid_authority(f"{field_name} must be a canonical User URA")
    user_id = projection.components.get("user_id")
    if not isinstance(user_id, str) or not user_id.strip():
        raise _invalid_authority(f"{field_name} must include a user id")
    return user_id.strip()


def _parse_required_ura(value: str, field_name: str) -> Any:
    try:
        return parse_ura(value.strip())
    except SDKError as exc:
        raise _invalid_authority(f"{field_name} must be a canonical URA", exc) from exc


def _canonical_ura_or_empty(value: str) -> str:
    value = value.strip()
    if not value:
        return ""
    try:
        parse_ura(value)
    except SDKError:
        return ""
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
        raise _invalid_authority(
            "delegation authority must bind issuer, subject, caller, and audience"
        )
    _reject_all_zero_authority_fields(
        {
            "issuer_ura": proof.issuer_ura,
            "subject_ura": proof.subject_ura,
            "caller_ura": proof.caller_ura,
            "audience": proof.audience,
        }
    )
    if not proof.scopes:
        raise _invalid_authority("delegation authority scopes are required")
    if proof.expires_at_ms <= proof.issued_at_ms:
        raise _invalid_authority(
            "delegation authority expires_at_ms must be greater than issued_at_ms"
        )
    if not proof.signature:
        raise _invalid_authority("delegation authority signature is required")


def _validate_session_authority(authority: SessionAuthority) -> None:
    authority = _session_authority_with_canonical_principals(authority)
    if not (
        authority.issuer_ura.strip()
        and authority.session_id.strip()
        and authority.session_owner_user_id.strip()
        and authority.creator_principal_id.strip()
        and authority.callee_ura.strip()
        and authority.subject_ura.strip()
        and authority.audience.strip()
    ):
        raise _invalid_authority(
            "session authority must bind issuer, session id, owner, creator principal, callee, subject, and audience"
        )
    _reject_all_zero_authority_fields(
        {
            "issuer_ura": authority.issuer_ura,
            "session_owner_user_id": authority.session_owner_user_id,
            "session_owner_ura": authority.session_owner_ura,
            "creator_principal_id": authority.creator_principal_id,
            "creator_principal_ura": authority.creator_principal_ura,
            "callee_ura": authority.callee_ura,
            "subject_ura": authority.subject_ura,
            "audience": authority.audience,
        }
    )
    if not authority.scopes or any(not scope.strip() for scope in authority.scopes):
        raise _invalid_authority("session authority scopes are required")
    if not authority.allowed_actions or any(
        not action.strip() for action in authority.allowed_actions
    ):
        raise _invalid_authority("session authority allowed actions are required")
    if not authority.allowed_followup_abilities or any(
        not ability.strip() for ability in authority.allowed_followup_abilities
    ):
        raise _invalid_authority(
            "session authority allowed follow-up abilities are required"
        )
    if authority.expires_at_ms <= authority.issued_at_ms:
        raise _invalid_authority(
            "session authority expires_at_ms must be greater than issued_at_ms"
        )
    if not authority.signature:
        raise _invalid_authority("session authority signature is required")


def _reject_all_zero_authority_fields(fields: Mapping[str, str]) -> None:
    for field_name, value in fields.items():
        if value and contains_all_zero_principal(value):
            raise _invalid_authority(f"{field_name} must not be all-zero")


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
    request = _normalized_session_authority_request(request)
    _validate_session_authority(
        SessionAuthority(
            issuer_ura=request.issuer_ura,
            session_id=request.session_id,
            session_owner_user_id=request.session_owner_user_id,
            creator_principal_id=request.creator_principal_id,
            callee_ura=request.callee_ura,
            subject_ura=request.subject_ura,
            audience=request.audience,
            scopes=tuple(request.scopes),
            allowed_actions=tuple(request.allowed_actions),
            allowed_followup_abilities=tuple(request.allowed_followup_abilities),
            issued_at_ms=request.issued_at_ms,
            expires_at_ms=request.expires_at_ms,
            signature=b"shape-only",
            session_owner_ura=request.session_owner_ura,
            creator_principal_ura=request.creator_principal_ura,
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
