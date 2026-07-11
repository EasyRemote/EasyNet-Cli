"""Public projections and capabilities for daemon-managed signing keys."""

from __future__ import annotations

import base64
import binascii
import hashlib
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass
from enum import StrEnum
from typing import Protocol, TypeVar

from ._key_service import (
    KeyServiceClient,
    MAX_KEY_SERVICE_CANONICAL_BYTES,
    decode_base64_value,
    invalid_key_service_input,
    invalid_key_service_payload,
    require_response_shape,
    required_response_bool,
    required_response_i64,
)
from .errors import ErrorCode, RetryHint, SDKError
from .identity import SignerHandle, _signer_handle_provenance_error
from .invocation import InvocationSignature
from .signing import SignatureProvider, SigningMaterial

DEFAULT_MANAGED_SIGNING_PAGE_SIZE = 16
MAX_MANAGED_SIGNING_PAGE_SIZE = 16
MAX_MANAGED_SIGNING_AUTO_PAGES = 1024
MAX_MANAGED_SIGNING_AUTO_ITEMS = 16_384

_MAX_CURSOR_BYTES = 4096
_I64_MAX = (1 << 63) - 1
_U64_MAX = (1 << 64) - 1


class ManagedSigningStatus(StrEnum):
    """Explicit lifecycle state for a rotatable managed signing key."""

    ACTIVE = "active"
    RETIRED = "retired"
    REVOKED = "revoked"


@dataclass(frozen=True)
class ManagedSigningKey:
    """Public projection of a daemon-custodied signing key."""

    key_id: str
    purpose: str
    public_key: bytes
    status: ManagedSigningStatus
    rotation_epoch: int
    bound_subject_ura: str | None
    signer_policy_ref: str | None
    rotated_from: str | None
    created_unix_ms: int
    expires_unix_ms: int | None
    revoked_unix_ms: int | None


@dataclass(frozen=True)
class ManagedSigningCreateRequest:
    purpose: str
    bound_subject_ura: str | None = None


@dataclass(frozen=True)
class ManagedSigningKeyFilter:
    purpose: str | None = None
    status: ManagedSigningStatus | None = None


@dataclass(frozen=True)
class ManagedSigningKeyPage:
    """One bounded page of public managed-key projections."""

    items: tuple[ManagedSigningKey, ...]
    next_cursor: str | None
    limit: int

    @property
    def keys(self) -> tuple[ManagedSigningKey, ...]:
        return self.items


@dataclass(frozen=True)
class ManagedSigningPeer:
    """Public trust projection for one peer runtime."""

    peer_ura: str
    fingerprint: bytes
    public_key: bytes
    via_hub_ura: str | None
    added_unix_ms: int
    last_seen_unix_ms: int


@dataclass(frozen=True)
class ManagedSigningPeerRegistration:
    peer_ura: str
    public_key: bytes
    via_hub_ura: str | None = None


@dataclass(frozen=True)
class ManagedSigningPeerPage:
    """One bounded page of public peer trust projections."""

    items: tuple[ManagedSigningPeer, ...]
    next_cursor: str | None
    limit: int

    @property
    def peers(self) -> tuple[ManagedSigningPeer, ...]:
        return self.items


@dataclass(frozen=True)
class ManagedSigningClient:
    """Provider-backed facade over the daemon managed-signing domain."""

    socket_path: str = ""
    timeout_seconds: float = 10.0

    def __post_init__(self) -> None:
        client = KeyServiceClient(self.socket_path, self.timeout_seconds)
        object.__setattr__(self, "socket_path", client.socket_path)
        object.__setattr__(self, "timeout_seconds", client.timeout_seconds)

    def create(self, request: ManagedSigningCreateRequest) -> ManagedSigningKey:
        if not isinstance(request, ManagedSigningCreateRequest):
            raise invalid_key_service_input(
                "managed signing create request is required"
            )
        purpose = _required_text("purpose", request.purpose)
        payload: dict[str, object] = {"method": "inventory.create", "purpose": purpose}
        subject_ura = None
        if request.bound_subject_ura is not None:
            subject_ura = _required_text("bound subject URA", request.bound_subject_ura)
            payload["bound_subject"] = subject_ura
        key = _decode_key_response(self._client.call(payload))
        if (
            key.purpose != purpose
            or key.bound_subject_ura != subject_ura
            or key.status != ManagedSigningStatus.ACTIVE
            or key.rotation_epoch != 0
            or key.rotated_from is not None
        ):
            raise invalid_key_service_payload(
                "daemon key service violated managed-key create postconditions"
            )
        return key

    def list_page(
        self,
        filter: ManagedSigningKeyFilter = ManagedSigningKeyFilter(),
        *,
        limit: int = DEFAULT_MANAGED_SIGNING_PAGE_SIZE,
        cursor: str | None = None,
    ) -> ManagedSigningKeyPage:
        limit = _page_limit(limit)
        payload = _key_filter_payload(filter)
        payload["limit"] = limit
        normalized_cursor = _optional_cursor(cursor)
        if normalized_cursor is not None:
            payload["cursor"] = normalized_cursor
        response = self._client.call(payload)
        require_response_shape(
            response,
            "inventory_keys",
            required=("entries", "next_cursor"),
        )
        raw_entries = response.get("entries")
        if not isinstance(raw_entries, list):
            raise invalid_key_service_payload(
                "daemon key-service response field entries must be an array"
            )
        if len(raw_entries) > limit:
            raise invalid_key_service_payload(
                "daemon key service returned more managed keys than the page limit"
            )
        keys = tuple(_decode_key(entry) for entry in raw_entries)
        _reject_duplicate_values((key.key_id for key in keys), "key IDs")
        if filter.purpose is not None:
            expected_purpose = _required_text("purpose filter", filter.purpose)
            if any(key.purpose != expected_purpose for key in keys):
                raise invalid_key_service_payload(
                    "daemon key service returned a key outside the purpose filter"
                )
        if filter.status is not None:
            expected_status = ManagedSigningStatus(filter.status)
            if any(key.status != expected_status for key in keys):
                raise invalid_key_service_payload(
                    "daemon key service returned a key outside the status filter"
                )
        next_cursor = _response_cursor(response.get("next_cursor"))
        if next_cursor is not None and next_cursor == normalized_cursor:
            raise invalid_key_service_payload(
                "daemon key service did not advance the managed-key cursor"
            )
        return ManagedSigningKeyPage(keys, next_cursor, limit)

    def list(
        self, filter: ManagedSigningKeyFilter = ManagedSigningKeyFilter()
    ) -> tuple[ManagedSigningKey, ...]:
        return _collect_pages(
            lambda cursor: self.list_page(filter, cursor=cursor),
            lambda key: key.key_id,
            "managed key IDs",
        )

    def public_projection(self, key_id: str) -> ManagedSigningKey:
        normalized_key_id = _required_text("key ID", key_id)
        response = self._client.call(
            {"method": "inventory.public_key", "key_id": normalized_key_id}
        )
        key = _decode_key_response(response)
        if key.key_id != normalized_key_id:
            raise invalid_key_service_payload(
                "daemon key service returned a different managed key projection"
            )
        return key

    def sign(self, key_id: str, canonical_bytes: bytes) -> bytes:
        """Compatibility entry point routed through a key-bound signer."""

        normalized_key_id = _required_text("key ID", key_id)
        _validate_canonical_bytes(canonical_bytes)
        return self.signer(normalized_key_id).sign_canonical(canonical_bytes)

    def signer(self, key_id: str) -> "ManagedSigner":
        """Bind future signatures to one checked public-key projection."""

        key = self.public_projection(key_id)
        if key.status != ManagedSigningStatus.ACTIVE:
            raise _policy_error("only active managed signing keys can create a signer")
        if key.bound_subject_ura is None or key.signer_policy_ref is None:
            raise _policy_error(
                "managed signing key must be bound to a subject before creating a signer"
            )
        return ManagedSigner(
            key=key,
            socket_path=self.socket_path,
            timeout_seconds=self.timeout_seconds,
        )

    def rotate(self, key_id: str) -> ManagedSigningKey:
        predecessor = _required_text("key ID", key_id)
        response = self._client.call(
            {"method": "inventory.rotate", "key_id": predecessor}
        )
        successor = _decode_key_response(response)
        if (
            successor.status != ManagedSigningStatus.ACTIVE
            or successor.rotated_from != predecessor
            or successor.rotation_epoch == 0
            or successor.key_id == predecessor
        ):
            raise invalid_key_service_payload(
                "daemon key service violated managed-key rotation postconditions"
            )
        return successor

    def revoke(self, key_id: str) -> int:
        response = self._client.call(
            {"method": "inventory.revoke", "key_id": _required_text("key ID", key_id)}
        )
        require_response_shape(
            response, "inventory_revoked", required=("revoked_unix_ms",)
        )
        revoked_unix_ms = required_response_i64(response, "revoked_unix_ms")
        if revoked_unix_ms <= 0:
            raise invalid_key_service_payload(
                "managed signing revoke timestamp must be positive"
            )
        return revoked_unix_ms

    def set_expiry(self, key_id: str, expires_unix_ms: int) -> None:
        expires_unix_ms = _positive_input_i64("managed signing expiry", expires_unix_ms)
        response = self._client.call(
            {
                "method": "inventory.set_expiry",
                "key_id": _required_text("key ID", key_id),
                "expires_unix_ms": expires_unix_ms,
            }
        )
        require_response_shape(response, "ok")

    def bind_subject(self, key_id: str, subject_ura: str) -> None:
        response = self._client.call(
            {
                "method": "inventory.bind_subject",
                "key_id": _required_text("key ID", key_id),
                "subject_ura": _required_text("subject URA", subject_ura),
            }
        )
        require_response_shape(response, "ok")

    def add_peer(self, registration: ManagedSigningPeerRegistration) -> bool:
        if not isinstance(registration, ManagedSigningPeerRegistration):
            raise invalid_key_service_input(
                "managed signing peer registration is required"
            )
        peer_ura = _required_text("peer URA", registration.peer_ura)
        if (
            not isinstance(registration.public_key, bytes)
            or len(registration.public_key) != 32
        ):
            raise invalid_key_service_input(
                "managed signing peer public key must be 32 bytes"
            )
        payload: dict[str, object] = {
            "method": "inventory.peer_add",
            "peer_ura": peer_ura,
            "public_key_b64": base64.b64encode(registration.public_key).decode("ascii"),
        }
        if registration.via_hub_ura is not None:
            payload["via_hub"] = _required_text(
                "peer via-hub URA", registration.via_hub_ura
            )
        response = self._client.call(payload)
        require_response_shape(response, "inventory_peer_added", required=("added",))
        return required_response_bool(response, "added")

    def list_peers_page(
        self,
        *,
        limit: int = DEFAULT_MANAGED_SIGNING_PAGE_SIZE,
        cursor: str | None = None,
    ) -> ManagedSigningPeerPage:
        limit = _page_limit(limit)
        payload: dict[str, object] = {
            "method": "inventory.peer_list",
            "limit": limit,
        }
        normalized_cursor = _optional_cursor(cursor)
        if normalized_cursor is not None:
            payload["cursor"] = normalized_cursor
        response = self._client.call(payload)
        require_response_shape(
            response,
            "inventory_peers",
            required=("peers", "next_cursor"),
        )
        raw_peers = response.get("peers")
        if not isinstance(raw_peers, list):
            raise invalid_key_service_payload(
                "daemon key-service response field peers must be an array"
            )
        if len(raw_peers) > limit:
            raise invalid_key_service_payload(
                "daemon key service returned more peers than the page limit"
            )
        peers = tuple(_decode_peer(peer) for peer in raw_peers)
        _reject_duplicate_values((peer.peer_ura for peer in peers), "peer URAs")
        next_cursor = _response_cursor(response.get("next_cursor"))
        if next_cursor is not None and next_cursor == normalized_cursor:
            raise invalid_key_service_payload(
                "daemon key service did not advance the peer cursor"
            )
        return ManagedSigningPeerPage(peers, next_cursor, limit)

    def list_peers(self) -> tuple[ManagedSigningPeer, ...]:
        return _collect_pages(
            lambda cursor: self.list_peers_page(cursor=cursor),
            lambda peer: peer.peer_ura,
            "managed peer URAs",
        )

    def _sign(self, key: ManagedSigningKey, canonical_bytes: bytes) -> bytes:
        _validate_canonical_bytes(canonical_bytes)
        if key.bound_subject_ura is None or key.signer_policy_ref is None:
            raise _policy_error(
                "managed signer requires a bound subject and signer policy reference"
            )
        response = self._client.call(
            {
                "method": "inventory.sign",
                "key_id": key.key_id,
                "expected_purpose": key.purpose,
                "subject_ura": key.bound_subject_ura,
                "signer_policy_ref": key.signer_policy_ref,
                "canonical_bytes_b64": base64.b64encode(canonical_bytes).decode(
                    "ascii"
                ),
            }
        )
        require_response_shape(response, "signature", required=("signature_b64",))
        return decode_base64_value(
            response.get("signature_b64"), "signature_b64", expected_len=64
        )

    @property
    def _client(self) -> KeyServiceClient:
        return KeyServiceClient(self.socket_path, self.timeout_seconds)


@dataclass(frozen=True)
class ManagedSigner(SignatureProvider):
    """Key-bound canonical signer and structural ``SignatureProvider``."""

    key: ManagedSigningKey
    socket_path: str
    timeout_seconds: float = 10.0

    @property
    def key_id(self) -> str:
        return self.key.key_id

    @property
    def public_key(self) -> bytes:
        return bytes(self.key.public_key)

    def signing_public_key(self) -> bytes:
        return self.public_key

    def sign_canonical(self, canonical_bytes: bytes) -> bytes:
        if self.key.status != ManagedSigningStatus.ACTIVE:
            raise _policy_error("only active managed signing keys can sign")
        signature = ManagedSigningClient(self.socket_path, self.timeout_seconds)._sign(
            self.key, canonical_bytes
        )
        _verify_ed25519_signature(self.key.public_key, canonical_bytes, signature)
        return signature

    def sign(
        self, material: SigningMaterial, handle: SignerHandle
    ) -> InvocationSignature:
        """Implement the SDK ``SignatureProvider`` contract."""

        if not isinstance(material, SigningMaterial) or not isinstance(
            handle, SignerHandle
        ):
            raise invalid_key_service_input(
                "managed signer requires signing material and a signer handle"
            )
        provenance_error = _signer_handle_provenance_error(handle)
        if provenance_error:
            raise invalid_key_service_input(provenance_error)
        if (
            material.algorithm.lower() != "ed25519"
            or handle.algorithm.lower() != "ed25519"
        ):
            raise invalid_key_service_input("managed signer requires ed25519")
        if handle.key_id != self.key.key_id:
            raise invalid_key_service_input(
                "signer handle key ID does not match the managed signer"
            )
        subject_ura = self.key.bound_subject_ura
        policy_ref = self.key.signer_policy_ref
        if subject_ura is None or policy_ref is None:
            raise invalid_key_service_input(
                "managed SignatureProvider requires a subject-bound key"
            )
        if handle.owner_ura != subject_ura:
            raise invalid_key_service_input(
                "signer handle owner URA does not match the managed key subject"
            )
        if handle.policy.get("policy_ref") != policy_ref:
            raise invalid_key_service_input(
                "signer handle policy_ref does not match the managed key"
            )
        metadata_policy_ref = handle.metadata.get("policy_ref")
        if metadata_policy_ref is not None and metadata_policy_ref != policy_ref:
            raise invalid_key_service_input(
                "signer handle metadata policy_ref does not match the managed key"
            )
        expected_public_key = base64.b64encode(self.key.public_key).decode("ascii")
        metadata_public_key = handle.metadata.get("public_key_base64")
        if (
            metadata_public_key is not None
            and metadata_public_key != expected_public_key
        ):
            raise invalid_key_service_input(
                "signer handle public key does not match the managed key"
            )
        if material.signer_policy is not None:
            if material.signer_policy.policy_ref != policy_ref:
                raise invalid_key_service_input(
                    "prepared signer policy_ref does not match the managed key"
                )
            if (
                material.signer_policy.signer_id
                and material.signer_policy.signer_id != handle.signer_id
            ):
                raise invalid_key_service_input(
                    "prepared signer ID does not match the signer handle"
                )
        canonical_bytes = _decode_canonical_input(material.canonical_bytes_base64)
        signature = self.sign_canonical(canonical_bytes)
        return InvocationSignature(
            algorithm="ed25519",
            signature_base64=base64.b64encode(signature).decode("ascii"),
            key_id_hint=handle.signer_id,
            signer_public_key_base64=expected_public_key,
        )


def _key_filter_payload(filter: ManagedSigningKeyFilter) -> dict[str, object]:
    if not isinstance(filter, ManagedSigningKeyFilter):
        raise invalid_key_service_input("managed signing key filter is required")
    payload: dict[str, object] = {"method": "inventory.list"}
    if filter.purpose is not None:
        payload["purpose"] = _required_text("purpose filter", filter.purpose)
    if filter.status is not None:
        try:
            status = ManagedSigningStatus(filter.status)
        except (TypeError, ValueError) as exc:
            raise invalid_key_service_input(
                f"unsupported managed signing status {filter.status!r}", exc
            ) from exc
        payload["status"] = status.value
    return payload


def _decode_key_response(response: Mapping[str, object]) -> ManagedSigningKey:
    require_response_shape(response, "inventory_key", required=("entry",))
    return _decode_key(response.get("entry"))


def _decode_key(raw: object) -> ManagedSigningKey:
    if not isinstance(raw, Mapping):
        raise invalid_key_service_payload(
            "managed signing key projection must be an object"
        )
    _require_projection_fields(
        raw,
        (
            "key_id",
            "purpose",
            "public_key_b64",
            "status",
            "rotation_epoch",
            "bound_subject",
            "signer_policy_ref",
            "rotated_from",
            "created_unix_ms",
            "expires_unix_ms",
            "revoked_unix_ms",
        ),
        "managed signing key projection",
    )
    status_raw = _projection_text(raw, "status")
    try:
        status = ManagedSigningStatus(status_raw)
    except ValueError as exc:
        raise invalid_key_service_payload(
            f"daemon key service returned unsupported managed signing status {status_raw!r}",
            exc,
        ) from exc
    key_id = _projection_text(raw, "key_id")
    purpose = _projection_text(raw, "purpose")
    public_key = decode_base64_value(
        raw.get("public_key_b64"), "public_key_b64", expected_len=32
    )
    rotation_epoch = _projection_u64(raw, "rotation_epoch")
    bound_subject = _optional_projection_text(raw, "bound_subject")
    policy_ref = _optional_projection_text(raw, "signer_policy_ref")
    rotated_from = _optional_projection_text(raw, "rotated_from")
    created_unix_ms = _positive_projection_i64(raw, "created_unix_ms")
    expires_unix_ms = _optional_positive_projection_i64(raw, "expires_unix_ms")
    revoked_unix_ms = _optional_positive_projection_i64(raw, "revoked_unix_ms")

    if rotation_epoch == 0 and rotated_from is not None:
        raise invalid_key_service_payload(
            "genesis managed signing key must not declare rotated_from"
        )
    if rotation_epoch > 0 and rotated_from is None:
        raise invalid_key_service_payload(
            "rotated managed signing key must declare its predecessor"
        )
    if rotated_from == key_id:
        raise invalid_key_service_payload(
            "managed signing key cannot rotate from itself"
        )
    if status == ManagedSigningStatus.REVOKED:
        if revoked_unix_ms is None or revoked_unix_ms < created_unix_ms:
            raise invalid_key_service_payload(
                "revoked managed signing key requires a valid terminal timestamp"
            )
    elif revoked_unix_ms is not None:
        raise invalid_key_service_payload(
            "non-revoked managed signing key cannot have a revoke timestamp"
        )

    expected_policy_ref = (
        _canonical_signer_policy_ref(purpose, bound_subject, key_id, public_key)
        if bound_subject is not None
        else None
    )
    if policy_ref != expected_policy_ref:
        raise invalid_key_service_payload(
            "managed signing signer_policy_ref is not canonically bound"
        )

    return ManagedSigningKey(
        key_id=key_id,
        purpose=purpose,
        public_key=public_key,
        status=status,
        rotation_epoch=rotation_epoch,
        bound_subject_ura=bound_subject,
        signer_policy_ref=policy_ref,
        rotated_from=rotated_from,
        created_unix_ms=created_unix_ms,
        expires_unix_ms=expires_unix_ms,
        revoked_unix_ms=revoked_unix_ms,
    )


def _decode_peer(raw: object) -> ManagedSigningPeer:
    if not isinstance(raw, Mapping):
        raise invalid_key_service_payload(
            "managed signing peer projection must be an object"
        )
    _require_projection_fields(
        raw,
        (
            "peer_ura",
            "fingerprint_b64",
            "public_key_b64",
            "via_hub",
            "added_unix_ms",
            "last_seen_unix_ms",
        ),
        "managed signing peer projection",
    )
    public_key = decode_base64_value(
        raw.get("public_key_b64"), "public_key_b64", expected_len=32
    )
    fingerprint = decode_base64_value(
        raw.get("fingerprint_b64"), "fingerprint_b64", expected_len=32
    )
    if fingerprint != hashlib.sha256(public_key).digest():
        raise invalid_key_service_payload(
            "managed signing peer fingerprint does not match its public key"
        )
    added_unix_ms = _positive_projection_i64(raw, "added_unix_ms")
    last_seen_unix_ms = _positive_projection_i64(raw, "last_seen_unix_ms")
    if last_seen_unix_ms < added_unix_ms:
        raise invalid_key_service_payload(
            "managed signing peer last_seen precedes added timestamp"
        )
    return ManagedSigningPeer(
        peer_ura=_projection_text(raw, "peer_ura"),
        fingerprint=fingerprint,
        public_key=public_key,
        via_hub_ura=_optional_projection_text(raw, "via_hub"),
        added_unix_ms=added_unix_ms,
        last_seen_unix_ms=last_seen_unix_ms,
    )


def _canonical_signer_policy_ref(
    purpose: str, subject_ura: str, key_id: str, public_key: bytes
) -> str:
    public_key_base64 = base64.b64encode(public_key).decode("ascii")
    digest = hashlib.sha256()
    for component in (
        "canonical-runtime.managed-signing.policy",
        "v2",
        purpose,
        subject_ura,
        key_id,
        public_key_base64,
    ):
        digest.update(component.encode("utf-8"))
        digest.update(b"\0")
    return f"managed-signing:v2:sha256:{digest.hexdigest()[:32]}"


def _verify_ed25519_signature(
    public_key: bytes, canonical_bytes: bytes, signature: bytes
) -> None:
    try:
        from cryptography.exceptions import InvalidSignature
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    except ImportError as exc:
        raise invalid_key_service_payload(
            "Ed25519 verifier is unavailable; cryptography is required to validate daemon signatures",
            exc,
        ) from exc
    try:
        Ed25519PublicKey.from_public_bytes(public_key).verify(
            signature, canonical_bytes
        )
    except InvalidSignature as exc:
        raise invalid_key_service_payload(
            "daemon key service returned an invalid managed signature", exc
        ) from exc
    except ValueError as exc:
        raise invalid_key_service_payload(
            "managed signer public key is invalid", exc
        ) from exc


def _decode_canonical_input(value: str) -> bytes:
    if not isinstance(value, str) or not value:
        raise invalid_key_service_input("canonical signing material is required")
    try:
        decoded = base64.b64decode(value, validate=True)
    except (binascii.Error, ValueError) as exc:
        raise invalid_key_service_input(
            "canonical signing material must be base64", exc
        ) from exc
    if not decoded:
        raise invalid_key_service_input("canonical signing material is required")
    if len(decoded) > MAX_KEY_SERVICE_CANONICAL_BYTES:
        raise invalid_key_service_input(
            "canonical bytes exceed the 64 MiB signing limit"
        )
    return decoded


def _validate_canonical_bytes(value: object) -> None:
    if not isinstance(value, bytes) or not value:
        raise invalid_key_service_input(
            "canonical bytes are required for managed signing"
        )
    if len(value) > MAX_KEY_SERVICE_CANONICAL_BYTES:
        raise invalid_key_service_input(
            "canonical bytes exceed the 64 MiB signing limit"
        )


_PageItem = TypeVar("_PageItem")


class _Page(Protocol[_PageItem]):
    items: tuple[_PageItem, ...]
    next_cursor: str | None


def _collect_pages(
    fetch: Callable[[str | None], _Page[_PageItem]],
    identity: Callable[[_PageItem], str],
    label: str,
) -> tuple[_PageItem, ...]:
    items: list[_PageItem] = []
    seen_items: set[str] = set()
    seen_cursors: set[str] = set()
    cursor: str | None = None
    for _ in range(MAX_MANAGED_SIGNING_AUTO_PAGES):
        page = fetch(cursor)
        page_items = page.items
        next_cursor = page.next_cursor
        for item in page_items:
            item_identity = identity(item)
            if item_identity in seen_items:
                raise invalid_key_service_payload(
                    f"daemon key service returned duplicate {label} across pages"
                )
            seen_items.add(item_identity)
            items.append(item)
            if len(items) > MAX_MANAGED_SIGNING_AUTO_ITEMS:
                raise invalid_key_service_payload(
                    "managed signing compatibility pagination exceeded its item bound"
                )
        if next_cursor is None:
            return tuple(items)
        if not isinstance(next_cursor, str) or next_cursor in seen_cursors:
            raise invalid_key_service_payload(
                "managed signing compatibility pagination repeated a cursor"
            )
        seen_cursors.add(next_cursor)
        cursor = next_cursor
    raise invalid_key_service_payload(
        "managed signing compatibility pagination exceeded its page bound"
    )


def _require_projection_fields(
    raw: Mapping[str, object], fields: tuple[str, ...], projection: str
) -> None:
    expected = set(fields)
    unknown = sorted(set(raw).difference(expected))
    missing = sorted(expected.difference(raw))
    if unknown:
        raise invalid_key_service_payload(
            f"{projection} contains unknown fields: " + ", ".join(unknown)
        )
    if missing:
        raise invalid_key_service_payload(
            f"{projection} is missing fields: " + ", ".join(missing)
        )


def _required_text(field: str, value: object) -> str:
    if not isinstance(value, str) or not value.strip():
        raise invalid_key_service_input(f"managed signing {field} is required")
    return value.strip()


def _projection_text(raw: Mapping[str, object], field: str) -> str:
    value = raw.get(field)
    if not isinstance(value, str) or not value or value.strip() != value:
        raise invalid_key_service_payload(
            f"daemon key service returned an invalid managed signing {field}"
        )
    return value


def _optional_projection_text(raw: Mapping[str, object], field: str) -> str | None:
    value = raw.get(field)
    if value is None:
        return None
    return _projection_text(raw, field)


def _projection_i64(raw: Mapping[str, object], field: str) -> int:
    value = raw.get(field)
    if (
        isinstance(value, bool)
        or not isinstance(value, int)
        or value < -(1 << 63)
        or value > _I64_MAX
    ):
        raise invalid_key_service_payload(
            f"daemon key service returned an invalid i64 managed signing {field}"
        )
    return value


def _projection_u64(raw: Mapping[str, object], field: str) -> int:
    value = raw.get(field)
    if (
        isinstance(value, bool)
        or not isinstance(value, int)
        or value < 0
        or value > _U64_MAX
    ):
        raise invalid_key_service_payload(
            f"daemon key service returned an invalid u64 managed signing {field}"
        )
    return value


def _positive_projection_i64(raw: Mapping[str, object], field: str) -> int:
    value = _projection_i64(raw, field)
    if value <= 0:
        raise invalid_key_service_payload(f"managed signing {field} must be positive")
    return value


def _optional_positive_projection_i64(
    raw: Mapping[str, object], field: str
) -> int | None:
    if raw.get(field) is None:
        return None
    return _positive_projection_i64(raw, field)


def _optional_positive_projection_i64(
    raw: Mapping[str, object], field: str
) -> int | None:
    if raw.get(field) is None:
        return None
    value = _projection_i64(raw, field)
    if value <= 0:
        raise invalid_key_service_payload(
            f"managed signing {field} must be a positive i64"
        )
    return value


def _positive_input_i64(field: str, value: object) -> int:
    if (
        isinstance(value, bool)
        or not isinstance(value, int)
        or value <= 0
        or value > _I64_MAX
    ):
        raise invalid_key_service_input(f"{field} must be a positive i64")
    return value


def _page_limit(limit: object) -> int:
    if (
        isinstance(limit, bool)
        or not isinstance(limit, int)
        or limit < 1
        or limit > MAX_MANAGED_SIGNING_PAGE_SIZE
    ):
        raise invalid_key_service_input(
            f"managed signing page limit must be within 1..{MAX_MANAGED_SIGNING_PAGE_SIZE}"
        )
    return limit


def _optional_cursor(cursor: object) -> str | None:
    if cursor is None:
        return None
    if not isinstance(cursor, str) or not cursor or cursor.strip() != cursor:
        raise invalid_key_service_input(
            "managed signing cursor must be a non-empty clean string"
        )
    if len(cursor.encode("utf-8")) > _MAX_CURSOR_BYTES:
        raise invalid_key_service_input("managed signing cursor exceeds 4096 bytes")
    return cursor


def _response_cursor(cursor: object) -> str | None:
    if cursor is None:
        return None
    try:
        return _optional_cursor(cursor)
    except SDKError as exc:
        raise invalid_key_service_payload(
            "daemon key service returned an invalid pagination cursor", exc
        ) from exc


def _reject_duplicate_values(values: Iterable[str], label: str) -> None:
    materialized = tuple(values)
    if len(set(materialized)) != len(materialized):
        raise invalid_key_service_payload(
            f"daemon key service returned duplicate {label}"
        )


def _policy_error(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.POLICY_DENIED,
        stage="key_service",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )
