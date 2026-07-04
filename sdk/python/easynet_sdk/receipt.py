"""Receipt projection facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Mapping, Optional, Protocol, Sequence, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError
from ._lifecycle import ClientLifecycle


@dataclass(frozen=True)
class ReceiptFetchRequest:
    """Complete carrier context for receipt fetch."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    invocation_ura: str = ""
    request_id: str = ""
    trace_id: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_bytes(self) -> bytes:
        _validate_fetch_request(self)
        value: dict[str, object] = {
            "caller_ura": self.caller_ura,
            "callee_ura": self.callee_ura,
            "subject_ura": self.subject_ura,
            "descriptor_version": self.descriptor_version,
            "nonce_base64": self.nonce_base64,
            "causal_context": dict(self.causal_context),
        }
        if self.invocation_ura:
            value["invocation_ura"] = self.invocation_ura
        if self.request_id:
            value["request_id"] = self.request_id
        if self.trace_id:
            value["trace_id"] = self.trace_id
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


@dataclass(frozen=True)
class ReceiptChainVerificationRequest:
    """Ordered receipt bodies for daemon/Axon continuity checks."""

    receipts: Sequence[bytes | str]
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_bytes(self) -> bytes:
        receipts = _validate_chain_request(self)
        value: dict[str, object] = {"receipts": receipts}
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


@dataclass(frozen=True)
class ReceiptSummary:
    """SDK receipt.schema.json projection."""

    state: str
    verified: bool
    output: Any
    receipt_ura: Optional[str] = None
    invocation_id: Optional[str] = None
    error: Optional[SDKError] = None
    causal_ref: Optional[str] = None
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "ReceiptSummary":
        decoded = _json_object(raw, "receipt summary")
        if "output" not in decoded:
            raise _invalid_receipt("output is required")
        return cls(
            state=_required_string(decoded, "state"),
            verified=_required_bool(decoded, "verified"),
            output=decoded.get("output"),
            receipt_ura=_optional_string(decoded.get("receipt_ura"), "receipt_ura"),
            invocation_id=_optional_string(decoded.get("invocation_id"), "invocation_id"),
            error=_optional_sdk_error(decoded.get("error"), "error"),
            causal_ref=_optional_string(decoded.get("causal_ref"), "causal_ref"),
            metadata=_optional_mapping(decoded.get("metadata"), "metadata") or {},
        )


@dataclass(frozen=True)
class ReceiptChainItemVerification:
    """One daemon-projected receipt-chain edge."""

    index: int
    receipt_ura: str
    receipt_hash_hex: str
    continuous: bool
    invocation_id: Optional[str] = None
    prev_receipt_hash_hex: Optional[str] = None
    reason: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_mapping(
        cls, decoded: Mapping[str, object], expected_index: int
    ) -> "ReceiptChainItemVerification":
        index = _required_int(decoded, "index")
        if index != expected_index:
            raise _invalid_receipt("chain item index must match position")
        return cls(
            index=index,
            receipt_ura=_required_string(decoded, "receipt_ura"),
            invocation_id=_optional_string(decoded.get("invocation_id"), "invocation_id"),
            receipt_hash_hex=_required_string(decoded, "receipt_hash_hex"),
            prev_receipt_hash_hex=_optional_string(
                decoded.get("prev_receipt_hash_hex"), "prev_receipt_hash_hex"
            ),
            continuous=_required_bool(decoded, "continuous"),
            reason=_optional_string(decoded.get("reason"), "reason") or "",
            metadata=_optional_mapping(decoded.get("metadata"), "metadata") or {},
        )


@dataclass(frozen=True)
class ReceiptVerification:
    """Daemon/Axon receipt verification projection."""

    verified: bool
    method: str
    receipt_ura: Optional[str] = None
    invocation_id: Optional[str] = None
    reason: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "ReceiptVerification":
        decoded = _json_object(raw, "receipt verification")
        return cls(
            verified=_required_bool(decoded, "verified"),
            method=_required_string(decoded, "method"),
            receipt_ura=_optional_string(decoded.get("receipt_ura"), "receipt_ura"),
            invocation_id=_optional_string(decoded.get("invocation_id"), "invocation_id"),
            reason=_optional_string(decoded.get("reason"), "reason") or "",
            metadata=_optional_mapping(decoded.get("metadata"), "metadata") or {},
        )


@dataclass(frozen=True)
class ReceiptChainVerification:
    """Daemon/Axon receipt-chain continuity projection."""

    verified: bool
    continuous: bool
    method: str
    requires_full_receipt: bool
    receipt_count: int
    items: tuple[ReceiptChainItemVerification, ...]
    root_receipt_ura: Optional[str] = None
    terminal_receipt_ura: Optional[str] = None
    reason: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "ReceiptChainVerification":
        decoded = _json_object(raw, "receipt chain verification")
        raw_items = decoded.get("items")
        if not isinstance(raw_items, list) or not raw_items:
            raise _invalid_receipt("chain verification items are required")
        items: list[ReceiptChainItemVerification] = []
        for index, value in enumerate(raw_items):
            if not isinstance(value, dict):
                raise _invalid_receipt("chain verification item must be an object")
            items.append(ReceiptChainItemVerification.from_mapping(value, index))
        receipt_count = _required_int(decoded, "receipt_count")
        if receipt_count != len(items):
            raise _invalid_receipt("receipt_count must match items length")
        return cls(
            verified=_required_bool(decoded, "verified"),
            continuous=_required_bool(decoded, "continuous"),
            method=_required_string(decoded, "method"),
            requires_full_receipt=_required_bool(decoded, "requires_full_receipt"),
            root_receipt_ura=_optional_string(
                decoded.get("root_receipt_ura"), "root_receipt_ura"
            ),
            terminal_receipt_ura=_optional_string(
                decoded.get("terminal_receipt_ura"), "terminal_receipt_ura"
            ),
            receipt_count=receipt_count,
            items=tuple(items),
            reason=_optional_string(decoded.get("reason"), "reason") or "",
            metadata=_optional_mapping(decoded.get("metadata"), "metadata") or {},
        )


@dataclass(frozen=True)
class CausalRef:
    """Daemon/Axon-returned causal reference for child invocations."""

    causal_ref: str
    receipt_ura: Optional[str] = None
    invocation_id: Optional[str] = None
    form: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "CausalRef":
        decoded = _json_object(raw, "causal ref")
        return cls(
            causal_ref=_required_string(decoded, "causal_ref"),
            receipt_ura=_optional_string(decoded.get("receipt_ura"), "receipt_ura"),
            invocation_id=_optional_string(decoded.get("invocation_id"), "invocation_id"),
            form=_optional_string(decoded.get("form"), "form") or "",
            metadata=_optional_mapping(decoded.get("metadata"), "metadata") or {},
        )


@runtime_checkable
class ReceiptTransport(Protocol):
    """Concrete receipt operations supplied by the integration layer."""

    def fetch(self, request_json: bytes) -> bytes:
        ...

    def project(self, receipt_json: bytes) -> bytes:
        ...

    def verify(self, receipt_json: bytes) -> bytes:
        ...

    def verify_chain(self, request_json: bytes) -> bytes:
        ...

    def causal_ref(self, receipt_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class ReceiptClient:
    """Receipt profile facade."""

    transport: ReceiptTransport
    _lifecycle: ClientLifecycle = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_receipt("receipt transport is required")
        object.__setattr__(self, "_lifecycle", ClientLifecycle("receipt"))

    def fetch(self, request: ReceiptFetchRequest) -> ReceiptSummary:
        self._require_open()
        try:
            raw = self.transport.fetch(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("receipt fetch failed", exc) from exc
        return ReceiptSummary.from_json(raw)

    def project(self, receipt_json: bytes) -> ReceiptSummary:
        self._require_open()
        if not receipt_json:
            raise _invalid_receipt("receipt JSON is required")
        try:
            raw = self.transport.project(receipt_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("receipt project failed", exc) from exc
        return ReceiptSummary.from_json(raw)

    def verify(self, receipt_json: bytes) -> ReceiptVerification:
        self._require_open()
        if not receipt_json:
            raise _invalid_receipt("receipt JSON is required")
        try:
            raw = self.transport.verify(receipt_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("receipt verify failed", exc) from exc
        return ReceiptVerification.from_json(raw)

    def verify_chain(
        self, request: ReceiptChainVerificationRequest
    ) -> ReceiptChainVerification:
        self._require_open()
        try:
            raw = self.transport.verify_chain(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("receipt verify-chain failed", exc) from exc
        return ReceiptChainVerification.from_json(raw)

    def causal_ref(self, receipt_json: bytes) -> CausalRef:
        self._require_open()
        if not receipt_json:
            raise _invalid_receipt("receipt JSON is required")
        try:
            raw = self.transport.causal_ref(receipt_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("receipt causal-ref failed", exc) from exc
        return CausalRef.from_json(raw)

    def close(self) -> None:
        self._lifecycle.close(self.transport)

    def _require_open(self) -> None:
        self._lifecycle.require_open()


def _validate_fetch_request(request: ReceiptFetchRequest) -> None:
    if (
        not request.caller_ura
        or not request.callee_ura
        or not request.subject_ura
        or not request.descriptor_version
        or not request.nonce_base64
    ):
        raise _invalid_receipt(
            "caller_ura, callee_ura, subject_ura, descriptor_version, and nonce_base64 are required"
        )
    if request.causal_context is None:
        raise _invalid_receipt("causal_context is required")
    keys = sum(
        1
        for value in (request.invocation_ura, request.request_id, request.trace_id)
        if value
    )
    if keys != 1:
        raise _invalid_receipt("exactly one receipt lookup key is required")


def _validate_chain_request(
    request: ReceiptChainVerificationRequest,
) -> list[dict[str, object]]:
    if not request.receipts:
        raise _invalid_receipt("at least one receipt is required")
    receipts: list[dict[str, object]] = []
    seen_uras: set[str] = set()
    seen_hashes: set[str] = set()
    for index, raw in enumerate(request.receipts):
        if not raw:
            raise _invalid_receipt(f"receipt[{index}] JSON is required")
        decoded = _json_object(raw, f"receipt[{index}]")
        receipt_ura = _string_field(decoded, "receipt_ura")
        if receipt_ura:
            if receipt_ura in seen_uras:
                raise _invalid_receipt("duplicate receipt_ura in chain request")
            seen_uras.add(receipt_ura)
        receipt_hash = _receipt_hash_field(decoded)
        if receipt_hash:
            try:
                digest = bytes.fromhex(receipt_hash)
            except ValueError as exc:
                raise _invalid_receipt(
                    "receipt hash must decode to exactly 32 bytes", exc
                ) from exc
            if len(digest) != 32:
                raise _invalid_receipt("receipt hash must decode to exactly 32 bytes")
            if receipt_hash in seen_hashes:
                raise _invalid_receipt("duplicate receipt hash in chain request")
            seen_hashes.add(receipt_hash)
        receipts.append(decoded)
    return receipts


def _json_object(raw: bytes | str, label: str) -> dict[str, object]:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_receipt(f"decode {label} JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_receipt(f"{label} JSON must be an object")
    return decoded


def _string_field(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str):
        return ""
    return value.strip()


def _receipt_hash_field(decoded: Mapping[str, object]) -> str:
    for field_name in ("self_hash_hex", "receipt_hash_hex", "receipt_hash"):
        value = _string_field(decoded, field_name)
        if not value:
            continue
        value = value.removeprefix("sha256:").strip().lower()
        if value:
            return value
    return ""


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_receipt(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_receipt(f"{field_name} must be a string or null")
    return value


def _required_bool(decoded: Mapping[str, object], field_name: str) -> bool:
    value = decoded.get(field_name)
    if not isinstance(value, bool):
        raise _invalid_receipt(f"{field_name} must be a boolean")
    return value


def _required_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool):
        raise _invalid_receipt(f"{field_name} must be an integer")
    return value


def _optional_mapping(value: object, field_name: str) -> Optional[Mapping[str, object]]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_receipt(f"{field_name} must be an object or null")
    return dict(value)


def _optional_sdk_error(value: object, field_name: str) -> Optional[SDKError]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_receipt(f"{field_name} must be an object or null")
    return SDKError.from_json(json.dumps(value, separators=(",", ":"), sort_keys=True))


def _invalid_receipt(
    message: str, cause: BaseException | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="receipt",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )


def _transport_error(message: str, cause: BaseException) -> SDKError:
    return SDKError(
        code=ErrorCode.TRANSPORT,
        stage="transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=message,
        cause=cause,
    )
