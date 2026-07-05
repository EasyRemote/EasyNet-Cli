"""Receipt projection facade."""

from __future__ import annotations

import json
from collections.abc import Iterator
from dataclasses import dataclass, field
from enum import IntEnum
from typing import Any, Callable, Mapping, Optional, Protocol, Sequence, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError, profile_error_details
from ._lifecycle import ClientLifecycle
from .invocation import InvocationBuilder, InvocationDraft
from .runtime import InvocationResult, RuntimeReceipt


_PROFILE = "receipt"
_EASYREMOTE_RECEIPT_PROFILE = "easyremote_receipt"
_FETCH_ABILITY = "invocation.history.get"


@dataclass(frozen=True)
class ReceiptCarrierBase:
    """Complete Invocation carrier context shared by receipt read models."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    timeout_ms: int = 0
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_dict(self) -> dict[str, object]:
        _validate_carrier_base(self)
        value: dict[str, object] = {
            "caller_ura": self.caller_ura,
            "callee_ura": self.callee_ura,
            "subject_ura": self.subject_ura,
            "descriptor_version": self.descriptor_version,
            "nonce_base64": self.nonce_base64,
            "causal_context": dict(self.causal_context),
        }
        if self.timeout_ms:
            value["timeout_ms"] = self.timeout_ms
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return value


@dataclass(frozen=True)
class ReceiptHistoryReadRequest:
    """Daemon invocation-history/trace read-model request."""

    carrier: ReceiptCarrierBase
    arguments: Mapping[str, object] = field(default_factory=dict)

    def to_json_bytes(self) -> bytes:
        if self.carrier is None:
            raise _invalid_receipt("receipt carrier is required")
        value = self.carrier.to_json_dict()
        if self.arguments:
            value["arguments"] = dict(self.arguments)
        return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


@dataclass(frozen=True)
class ReceiptFetchRequest:
    """Complete carrier context for receipt fetch."""

    caller_ura: str
    callee_ura: str
    descriptor_ref: str
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
            "descriptor_ref": self.descriptor_ref,
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
        verification = cls(
            verified=_required_bool(decoded, "verified"),
            method=_required_string(decoded, "method"),
            receipt_ura=_optional_string(decoded.get("receipt_ura"), "receipt_ura"),
            invocation_id=_optional_string(decoded.get("invocation_id"), "invocation_id"),
            reason=_optional_string(decoded.get("reason"), "reason") or "",
            metadata=_optional_mapping(decoded.get("metadata"), "metadata") or {},
        )
        if verification.verified and _is_summary_only_method(verification.method):
            raise _invalid_receipt("summary-only projection cannot be verified")
        return verification

    @property
    def is_cryptographic(self) -> bool:
        """Return whether this projection names Axon/full receipt verification."""

        if not self.verified:
            return False
        method = self.method.strip().lower().replace("_", "-")
        source = str(self.metadata.get("source", "")).strip().lower()
        assurance = str(self.metadata.get("assurance", "")).strip().lower()
        if method in {"summary-only", "daemon-receipt-chain-continuity"}:
            return False
        if assurance in {"cryptographic", "axon-cryptographic"}:
            return True
        if source in {"axon", "axon-verifier", "daemon-axon"}:
            return True
        return method.startswith("axon-") or method in {
            "full-receipt",
            "full-receipt-verification",
            "cryptographic",
        }

    def require_cryptographic(self) -> "ReceiptVerification":
        """Require Axon/full receipt verification evidence."""

        if self.is_cryptographic:
            return self
        raise _invalid_receipt(
            "receipt verification is not Axon-backed cryptographic evidence"
        )


@dataclass(frozen=True)
class ReceiptRef:
    """Opaque daemon/Axon-returned receipt anchor.

    The SDK validates shape only. It never constructs receipt URAs and never
    treats a hash pair as cryptographic verification.
    """

    receipt_ura: str
    receipt_hash_hex: str
    invocation_id: Optional[str] = None
    prev_receipt_hash_hex: Optional[str] = None
    index: Optional[int] = None
    metadata: Mapping[str, object] = field(default_factory=dict)

    def __post_init__(self) -> None:
        receipt_ura = self.receipt_ura.strip()
        if not receipt_ura:
            raise _invalid_receipt("receipt_ura is required")
        receipt_hash_hex = _normalize_receipt_hash(self.receipt_hash_hex)
        _validate_receipt_hash_hex(receipt_hash_hex)
        prev_hash = None
        if self.prev_receipt_hash_hex:
            prev_hash = _normalize_receipt_hash(self.prev_receipt_hash_hex)
            _validate_receipt_hash_hex(prev_hash)
        if self.index is not None and self.index < 0:
            raise _invalid_receipt("receipt index must be non-negative")
        object.__setattr__(self, "receipt_ura", receipt_ura)
        object.__setattr__(self, "receipt_hash_hex", receipt_hash_hex)
        object.__setattr__(self, "prev_receipt_hash_hex", prev_hash)
        object.__setattr__(self, "metadata", dict(self.metadata))

    @classmethod
    def from_json(cls, raw: bytes | str) -> "ReceiptRef":
        return cls.from_mapping(_json_object(raw, "receipt ref"))

    @classmethod
    def from_mapping(cls, decoded: Mapping[str, object]) -> "ReceiptRef":
        return cls(
            receipt_ura=_required_string(decoded, "receipt_ura"),
            receipt_hash_hex=_required_receipt_hash(decoded),
            invocation_id=_optional_string(decoded.get("invocation_id"), "invocation_id"),
            prev_receipt_hash_hex=_optional_receipt_hash(decoded),
            index=_optional_int(decoded.get("index"), "index"),
            metadata=_optional_mapping(decoded.get("metadata"), "metadata") or {},
        )

    @classmethod
    def from_runtime_receipt(cls, receipt: RuntimeReceipt) -> "ReceiptRef":
        if receipt is None:
            raise _invalid_receipt("runtime receipt summary is required")
        if not receipt.has_causal_anchor():
            raise _invalid_receipt("runtime receipt summary is missing causal anchor")
        return cls.from_mapping(receipt.to_json_dict())

    def to_json_dict(self) -> dict[str, object]:
        value: dict[str, object] = {
            "receipt_ura": self.receipt_ura,
            "receipt_hash_hex": self.receipt_hash_hex,
        }
        if self.invocation_id:
            value["invocation_id"] = self.invocation_id
        if self.prev_receipt_hash_hex:
            value["prev_receipt_hash_hex"] = self.prev_receipt_hash_hex
        if self.index is not None:
            value["index"] = self.index
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return value

    def to_json_bytes(self) -> bytes:
        return json.dumps(
            self.to_json_dict(), separators=(",", ":"), sort_keys=True
        ).encode("utf-8")

    def causal_context(self, client: "ReceiptClient") -> Mapping[str, object]:
        """Delegate receipt-to-causal-context projection through ReceiptClient."""

        if client is None:
            raise _invalid_receipt("receipt client is required")
        return client.causal_context(self.to_json_bytes())


class ReceiptChain(Sequence[ReceiptRef]):
    """Ordered receipt anchors whose verification is delegated to ReceiptClient."""

    def __init__(self, receipts: Sequence[ReceiptRef]) -> None:
        if not receipts:
            raise _invalid_receipt("at least one receipt ref is required")
        self._receipts = tuple(receipts)

    @classmethod
    def from_json_receipts(cls, receipts: Sequence[bytes | str]) -> "ReceiptChain":
        return cls(tuple(ReceiptRef.from_json(raw) for raw in receipts))

    @classmethod
    def from_mappings(cls, receipts: Sequence[Mapping[str, object]]) -> "ReceiptChain":
        return cls(tuple(ReceiptRef.from_mapping(raw) for raw in receipts))

    def __len__(self) -> int:
        return len(self._receipts)

    def __getitem__(self, index):
        return self._receipts[index]

    def __iter__(self):
        return iter(self._receipts)

    def to_json_receipts(self) -> tuple[bytes, ...]:
        return tuple(receipt.to_json_bytes() for receipt in self._receipts)

    def verify_continuity(
        self,
        client: "ReceiptClient",
        *,
        metadata: Mapping[str, object] | None = None,
    ) -> "ReceiptChainVerification":
        """Project continuity through the daemon Receipt facade."""

        if client is None:
            raise _invalid_receipt("receipt client is required")
        return client.verify_chain(
            ReceiptChainVerificationRequest(
                receipts=self.to_json_receipts(),
                metadata=metadata or {},
            )
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

    receipt_ura: str
    receipt_hash_hex: str
    causal_context: Mapping[str, object]
    causal_ref: str = ""
    invocation_id: Optional[str] = None
    verified: bool = False
    form: str = "scalar"
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "CausalRef":
        decoded = _json_object(raw, "causal ref")
        causal_context = _causal_context_from_projection(decoded)
        receipt_ura = _required_string(causal_context, "receipt_ura")
        receipt_hash_hex = _required_receipt_hash(causal_context)
        return cls(
            receipt_ura=receipt_ura,
            receipt_hash_hex=receipt_hash_hex,
            causal_context=causal_context,
            causal_ref=_optional_string(decoded.get("causal_ref"), "causal_ref") or "",
            invocation_id=_optional_string(decoded.get("invocation_id"), "invocation_id"),
            verified=_optional_bool(decoded.get("verified"), "verified") or False,
            form=_optional_string(causal_context.get("form"), "form") or "scalar",
            metadata=_optional_mapping(decoded.get("metadata"), "metadata") or {},
        )

    def to_causal_context(self) -> Mapping[str, object]:
        """Return the child-Invocation `causal_context` DTO."""

        return dict(self.causal_context)


class EasyRemoteInvocationState(IntEnum):
    """EasyRemote-facing invocation lifecycle states.

    Numbering is normative, from ``axon/v1/types.proto``. The SDK owns this
    projection so product clients do not carry Axon enum parsing code.
    """

    UNSPECIFIED = 0
    ACCEPTED = 1
    ADMITTED = 2
    DISPATCHED = 3
    RUNNING = 4
    COMPLETED = 5
    FAILED = 6
    TIMED_OUT = 7
    CANCELLED = 8

    @property
    def is_terminal(self) -> bool:
        return self in _EASYREMOTE_TERMINAL_STATES


_EASYREMOTE_TERMINAL_STATES = frozenset(
    {
        EasyRemoteInvocationState.COMPLETED,
        EasyRemoteInvocationState.FAILED,
        EasyRemoteInvocationState.TIMED_OUT,
        EasyRemoteInvocationState.CANCELLED,
    }
)

_EASYREMOTE_STATE_NAMES = {
    "unspecified": EasyRemoteInvocationState.UNSPECIFIED,
    "accepted": EasyRemoteInvocationState.ACCEPTED,
    "admitted": EasyRemoteInvocationState.ADMITTED,
    "dispatched": EasyRemoteInvocationState.DISPATCHED,
    "running": EasyRemoteInvocationState.RUNNING,
    "completed": EasyRemoteInvocationState.COMPLETED,
    "failed": EasyRemoteInvocationState.FAILED,
    "timed_out": EasyRemoteInvocationState.TIMED_OUT,
    "cancelled": EasyRemoteInvocationState.CANCELLED,
}


@dataclass(frozen=True)
class EasyRemoteReceipt:
    """EasyRemote receipt-summary facade.

    The daemon currently returns summary receipts on the local invoke path.
    This facade preserves the exact wire body while centralizing all parsing,
    hash-chain projection, and the honest full-receipt verification guardrail
    inside the SDK Receipt profile.
    """

    index: int
    invocation_id: str
    receipt_type: str
    state: EasyRemoteInvocationState
    timestamp_unix_ms: int
    prev_receipt_hash: bytes
    self_hash: bytes
    payload_content_type: str
    cleanup_complete: bool
    reason: str
    child_invocation_id: str
    raw: dict[str, Any] = field(repr=False)

    @classmethod
    def from_wire(cls, wire: Mapping[str, object]) -> "EasyRemoteReceipt":
        try:
            return cls(
                index=_easyremote_int(wire["index"]),
                invocation_id=str(wire["invocation_id"]),
                receipt_type=str(wire["receipt_type"]),
                state=_easyremote_parse_state(wire["state"]),
                timestamp_unix_ms=_easyremote_int(wire["timestamp_unix_ms"]),
                prev_receipt_hash=bytes.fromhex(str(wire["prev_receipt_hash_hex"])),
                self_hash=bytes.fromhex(str(wire["self_hash_hex"])),
                payload_content_type=str(wire.get("payload_content_type", "")),
                cleanup_complete=bool(wire.get("cleanup_complete", False)),
                reason=str(wire.get("reason", "")),
                child_invocation_id=str(wire.get("child_invocation_id", "")),
                raw=dict(wire),
            )
        except (KeyError, ValueError, TypeError) as exc:
            raise _easyremote_receipt_protocol_error(
                f"daemon receipt summary is malformed: {exc}", exc
            ) from exc

    def to_json_bytes(self) -> bytes:
        return json.dumps(
            self.raw, separators=(",", ":"), sort_keys=True
        ).encode("utf-8")

    def verify(
        self, client: "ReceiptClient | None" = None
    ) -> "ReceiptVerification":
        """Require Axon/full-receipt verification evidence.

        Summary-only receipts cannot satisfy this. When the backing transport
        cannot return cryptographic evidence, the SDK raises a typed
        ``full_receipt_unavailable`` error instead of silently accepting weaker
        proof.
        """

        verifier = client or ReceiptClient(LocalReceiptTransport())
        verification = verifier.verify(self.to_json_bytes())
        if verification.is_cryptographic:
            return verification
        raise _easyremote_full_receipt_unavailable(self.invocation_id)


class EasyRemoteReceiptChain(Sequence[EasyRemoteReceipt]):
    """Ordered EasyRemote receipt summaries with SDK-owned continuity checks."""

    def __init__(self, receipts: Sequence[EasyRemoteReceipt]) -> None:
        self._receipts = tuple(receipts)

    def __len__(self) -> int:
        return len(self._receipts)

    def __getitem__(self, index):
        return self._receipts[index]

    def __iter__(self) -> Iterator[EasyRemoteReceipt]:
        return iter(self._receipts)

    def verify_continuity(
        self,
        client: "ReceiptClient | None" = None,
    ) -> "ReceiptChainVerification | None":
        """Project hash-chain continuity through the SDK Receipt profile."""

        if len(self._receipts) < 2:
            return None
        verifier = client or ReceiptClient(LocalReceiptTransport())
        verification = verifier.verify_chain(
            ReceiptChainVerificationRequest(
                receipts=tuple(receipt.to_json_bytes() for receipt in self._receipts)
            )
        )
        if verification.continuous:
            return verification
        raise _easyremote_receipt_chain_broken(verification)


@runtime_checkable
class ReceiptTransport(Protocol):
    """Concrete receipt operations supplied by the integration layer."""

    def fetch(self, request_json: bytes) -> bytes:
        ...

    def build_fetch_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_list_history_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_get_history_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_trace_invocation(self, request_json: bytes) -> bytes:
        ...

    def list_history(self, request_json: bytes) -> bytes:
        ...

    def get_history(self, request_json: bytes) -> bytes:
        ...

    def get_trace(self, request_json: bytes) -> bytes:
        ...

    def project(self, receipt_json: bytes) -> bytes:
        ...

    def verify(self, receipt_json: bytes) -> bytes:
        ...

    def verify_chain(self, request_json: bytes) -> bytes:
        ...

    def causal_ref(self, receipt_json: bytes) -> bytes:
        ...


class LocalReceiptTransport:
    """Pure SDK receipt-summary projection transport.

    This transport owns local checks that can be performed on daemon receipt
    summaries. It never upgrades summaries into cryptographic verification and
    refuses causal refs unless the daemon/Axon receipt URA anchor is present.
    """

    def fetch(self, request_json: bytes) -> bytes:
        raise _invalid_receipt("local receipt transport cannot fetch receipts")

    def build_fetch_invocation(self, request_json: bytes) -> bytes:
        raise _invalid_receipt(
            "local receipt transport cannot build daemon fetch invocations"
        )

    def build_list_history_invocation(self, request_json: bytes) -> bytes:
        raise _invalid_receipt(
            "local receipt transport cannot build daemon history invocations"
        )

    def build_get_history_invocation(self, request_json: bytes) -> bytes:
        raise _invalid_receipt(
            "local receipt transport cannot build daemon history invocations"
        )

    def build_trace_invocation(self, request_json: bytes) -> bytes:
        raise _invalid_receipt(
            "local receipt transport cannot build daemon trace invocations"
        )

    def list_history(self, request_json: bytes) -> bytes:
        raise _invalid_receipt("local receipt transport cannot read daemon history")

    def get_history(self, request_json: bytes) -> bytes:
        raise _invalid_receipt("local receipt transport cannot read daemon history")

    def get_trace(self, request_json: bytes) -> bytes:
        raise _invalid_receipt("local receipt transport cannot read daemon traces")

    def project(self, receipt_json: bytes) -> bytes:
        receipt = _json_object(receipt_json, "receipt summary")
        return _json_bytes(_summary_projection(receipt))

    def verify(self, receipt_json: bytes) -> bytes:
        receipt = _json_object(receipt_json, "receipt summary")
        return _json_bytes(
            {
                "verified": False,
                "method": "summary-only",
                "receipt_ura": _optional_string(
                    receipt.get("receipt_ura"), "receipt_ura"
                ),
                "invocation_id": _optional_string(
                    receipt.get("invocation_id"), "invocation_id"
                ),
                "reason": "full receipt required",
                "metadata": {"source": "sdk_local_receipt"},
            }
        )

    def verify_chain(self, request_json: bytes) -> bytes:
        request = _json_object(request_json, "receipt chain request")
        raw_receipts = request.get("receipts")
        if not isinstance(raw_receipts, list) or not raw_receipts:
            raise _invalid_receipt("at least one receipt is required")
        receipts = [
            _json_object(
                raw if isinstance(raw, (bytes, str)) else json.dumps(raw),
                f"receipt[{index}]",
            )
            for index, raw in enumerate(raw_receipts)
        ]
        items: list[dict[str, object]] = []
        continuous = True
        reason = ""
        for index, receipt in enumerate(receipts):
            receipt_hash = _required_receipt_hash(receipt)
            prev_hash = _optional_receipt_hash(receipt)
            item_continuous = True
            if index > 0:
                previous_hash = _required_receipt_hash(receipts[index - 1])
                item_continuous = prev_hash == previous_hash
                if not item_continuous and not reason:
                    reason = f"receipt chain broken at index {index}"
                    continuous = False
            item: dict[str, object] = {
                "index": index,
                "receipt_ura": _receipt_ura_or_summary_anchor(receipt, index),
                "invocation_id": _optional_string(
                    receipt.get("invocation_id"), "invocation_id"
                ),
                "receipt_hash_hex": receipt_hash,
                "prev_receipt_hash_hex": prev_hash,
                "continuous": item_continuous,
                "reason": "" if item_continuous else reason,
                "metadata": {"source": "sdk_local_receipt"},
            }
            items.append(item)
        return _json_bytes(
            {
                "verified": False,
                "continuous": continuous,
                "method": "daemon_receipt_chain_continuity",
                "requires_full_receipt": True,
                "root_receipt_ura": items[0]["receipt_ura"],
                "terminal_receipt_ura": items[-1]["receipt_ura"],
                "receipt_count": len(items),
                "items": items,
                "reason": reason,
                "metadata": {"source": "sdk_local_receipt"},
            }
        )

    def causal_ref(self, receipt_json: bytes) -> bytes:
        receipt = _json_object(receipt_json, "receipt summary")
        receipt_ura = _required_string(receipt, "receipt_ura")
        receipt_hash = _required_receipt_hash(receipt)
        return _json_bytes(
            {
                "receipt_ura": receipt_ura,
                "receipt_hash_hex": receipt_hash,
                "verified": False,
                "causal_context": {
                    "form": "scalar",
                    "receipt_ura": receipt_ura,
                    "receipt_hash_hex": receipt_hash,
                },
                "causal_ref": f"receipt:{receipt_ura}",
                "invocation_id": _optional_string(
                    receipt.get("invocation_id"), "invocation_id"
                ),
                "form": "scalar",
                "metadata": {"source": "sdk_local_receipt"},
            }
        )

    def close(self) -> None:
        return None


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

    def build_fetch_invocation(self, request: ReceiptFetchRequest) -> InvocationDraft:
        """Project receipt fetch into the daemon invocation.history.get carrier."""

        self._require_open()
        try:
            raw = self.transport.build_fetch_invocation(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("receipt fetch invocation failed", exc) from exc
        return InvocationDraft.from_json(raw)

    def build_list_history_invocation(
        self, request: ReceiptHistoryReadRequest
    ) -> InvocationDraft:
        return self._build_history_invocation(
            request,
            self.transport.build_list_history_invocation,
            "receipt list-history invocation failed",
        )

    def build_get_history_invocation(
        self, request: ReceiptHistoryReadRequest
    ) -> InvocationDraft:
        return self._build_history_invocation(
            request,
            self.transport.build_get_history_invocation,
            "receipt get-history invocation failed",
        )

    def build_trace_invocation(self, request: ReceiptHistoryReadRequest) -> InvocationDraft:
        return self._build_history_invocation(
            request,
            self.transport.build_trace_invocation,
            "receipt trace invocation failed",
        )

    def list_history(self, request: ReceiptHistoryReadRequest) -> Mapping[str, object]:
        return self._read_history(
            request,
            self.transport.list_history,
            "receipt list-history failed",
            "receipt list-history output",
        )

    def get_history(self, request: ReceiptHistoryReadRequest) -> Mapping[str, object]:
        return self._read_history(
            request,
            self.transport.get_history,
            "receipt get-history failed",
            "receipt get-history output",
        )

    def get_trace(self, request: ReceiptHistoryReadRequest) -> Mapping[str, object]:
        return self._read_history(
            request,
            self.transport.get_trace,
            "receipt trace failed",
            "receipt trace output",
        )

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

    def causal_context(self, receipt_json: bytes) -> Mapping[str, object]:
        """Project raw receipt JSON into child Invocation causal context."""

        return self.causal_ref(receipt_json).to_causal_context()

    def causal_context_from_runtime_receipt(
        self, receipt: RuntimeReceipt
    ) -> Mapping[str, object]:
        """Project a terminal runtime receipt summary into child causal context."""

        self._require_open()
        if not receipt.has_causal_anchor():
            raise _invalid_receipt("runtime receipt summary is missing causal anchor")
        raw = json.dumps(
            receipt.to_json_dict(), separators=(",", ":"), sort_keys=True
        ).encode("utf-8")
        return self.causal_context(raw)

    def causal_context_from_invocation_result(
        self, result: InvocationResult
    ) -> Mapping[str, object]:
        """Project an invocation result receipt summary into child causal context."""

        self._require_open()
        if result.receipt_summary is None:
            raise _invalid_receipt("invocation result has no receipt summary")
        return self.causal_context_from_runtime_receipt(result.receipt_summary)

    def close(self) -> None:
        self._lifecycle.close(self.transport)

    def _require_open(self) -> None:
        self._lifecycle.require_open()

    def _build_history_invocation(
        self,
        request: ReceiptHistoryReadRequest,
        build: Callable[[bytes], bytes],
        message: str,
    ) -> InvocationDraft:
        self._require_open()
        try:
            raw = build(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(message, exc) from exc
        return InvocationDraft.from_json(raw)

    def _read_history(
        self,
        request: ReceiptHistoryReadRequest,
        read: Callable[[bytes], bytes],
        message: str,
        label: str,
    ) -> Mapping[str, object]:
        self._require_open()
        try:
            raw = read(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(message, exc) from exc
        return _json_object(raw, label)


def build_receipt_fetch_invocation(request: ReceiptFetchRequest) -> InvocationDraft:
    """Build a complete Runtime Core carrier for daemon receipt lookup."""

    _validate_fetch_request(request)
    metadata = dict(request.metadata)
    metadata["profile"] = _PROFILE
    metadata["system_ability"] = _FETCH_ABILITY
    metadata["carrier_owner"] = "daemon_sdk"
    return (
        InvocationBuilder()
        .with_caller_ura(request.caller_ura)
        .with_callee_ura(request.callee_ura)
        .with_descriptor_ref(request.descriptor_ref)
        .with_subject_ura(request.subject_ura)
        .with_nonce_base64(request.nonce_base64)
        .with_causal_context(request.causal_context)
        .with_json_args({"key": _receipt_fetch_key(request)})
        .with_content_type("application/json")
        .with_metadata(metadata)
        .build()
    )


def _validate_fetch_request(request: ReceiptFetchRequest) -> None:
    if (
        not request.caller_ura
        or not request.callee_ura
        or not request.descriptor_ref
        or not request.subject_ura
        or not request.descriptor_version
        or not request.nonce_base64
    ):
        raise _invalid_receipt(
            "caller_ura, callee_ura, descriptor_ref, subject_ura, descriptor_version, and nonce_base64 are required"
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


def _validate_carrier_base(request: ReceiptCarrierBase) -> None:
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
    if not isinstance(request.causal_context, Mapping):
        raise _invalid_receipt("causal_context must be an object")
    if not isinstance(request.metadata, Mapping):
        raise _invalid_receipt("metadata must be an object")
    if not isinstance(request.timeout_ms, int) or isinstance(request.timeout_ms, bool):
        raise _invalid_receipt("timeout_ms must be an integer")
    if request.timeout_ms < 0:
        raise _invalid_receipt("timeout_ms must be non-negative")


def _receipt_fetch_key(request: ReceiptFetchRequest) -> dict[str, object]:
    if request.invocation_ura:
        return {"invocation_ura": request.invocation_ura}
    if request.request_id:
        return {"request_id": request.request_id}
    if request.trace_id:
        return {"trace_id": request.trace_id}
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


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _summary_projection(receipt: Mapping[str, object]) -> dict[str, object]:
    state = receipt.get("state")
    return {
        "receipt_ura": _optional_string(receipt.get("receipt_ura"), "receipt_ura"),
        "invocation_id": _optional_string(receipt.get("invocation_id"), "invocation_id"),
        "state": str(state).lower() if state is not None else "unspecified",
        "verified": False,
        "output": receipt.get("output"),
        "error": None,
        "causal_ref": None,
        "metadata": {
            "source": "sdk_local_receipt",
            "receipt_hash_hex": _receipt_hash_field(receipt),
        },
    }


def _receipt_ura_or_summary_anchor(
    receipt: Mapping[str, object], index: int
) -> str:
    receipt_ura = _optional_string(receipt.get("receipt_ura"), "receipt_ura")
    if receipt_ura:
        return receipt_ura
    invocation_id = _optional_string(receipt.get("invocation_id"), "invocation_id")
    if invocation_id:
        return f"summary:{invocation_id}:{index}"
    return f"summary:index:{index}"


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


def _optional_receipt_hash(decoded: Mapping[str, object]) -> Optional[str]:
    for field_name in ("prev_receipt_hash_hex", "parent_receipt_hash_hex"):
        value = _string_field(decoded, field_name)
        if not value:
            continue
        value = _normalize_receipt_hash(value)
        _validate_receipt_hash_hex(value)
        return value
    return None


def _required_receipt_hash(decoded: Mapping[str, object]) -> str:
    receipt_hash = _receipt_hash_field(decoded)
    if not receipt_hash:
        raise _invalid_receipt("receipt_hash_hex is required")
    receipt_hash = _normalize_receipt_hash(receipt_hash)
    _validate_receipt_hash_hex(receipt_hash)
    return receipt_hash


def _normalize_receipt_hash(value: str) -> str:
    return value.removeprefix("sha256:").strip().lower()


def _is_summary_only_method(value: str) -> bool:
    return value.strip().lower().replace("_", "-") == "summary-only"


def _validate_receipt_hash_hex(value: str) -> None:
    try:
        digest = bytes.fromhex(value)
    except ValueError as exc:
        raise _invalid_receipt(
            "receipt hash must decode to exactly 32 bytes", exc
        ) from exc
    if len(digest) != 32:
        raise _invalid_receipt("receipt hash must decode to exactly 32 bytes")


def _causal_context_from_projection(
    decoded: Mapping[str, object],
) -> Mapping[str, object]:
    context = _optional_mapping(decoded.get("causal_context"), "causal_context")
    if context is not None:
        if not _optional_string(context.get("form"), "form"):
            raise _invalid_receipt("causal_context.form is required")
        _required_string(context, "receipt_ura")
        _required_receipt_hash(context)
        return dict(context)
    receipt_ura = _required_string(decoded, "receipt_ura")
    receipt_hash_hex = _required_receipt_hash(decoded)
    form = _optional_string(decoded.get("form"), "form") or "scalar"
    return {
        "form": form,
        "receipt_ura": receipt_ura,
        "receipt_hash_hex": receipt_hash_hex,
    }


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


def _optional_bool(value: object, field_name: str) -> Optional[bool]:
    if value is None:
        return None
    if not isinstance(value, bool):
        raise _invalid_receipt(f"{field_name} must be a boolean or null")
    return value


def _required_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool):
        raise _invalid_receipt(f"{field_name} must be an integer")
    return value


def _optional_int(value: object, field_name: str) -> Optional[int]:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool):
        raise _invalid_receipt(f"{field_name} must be an integer or null")
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


def _easyremote_parse_state(value: Any) -> EasyRemoteInvocationState:
    if isinstance(value, str):
        parsed = _EASYREMOTE_STATE_NAMES.get(value.strip().lower())
        if parsed is not None:
            return parsed
    try:
        return EasyRemoteInvocationState(int(value))
    except (ValueError, TypeError):
        return EasyRemoteInvocationState.UNSPECIFIED


def _easyremote_int(value: object) -> int:
    if isinstance(value, bool):
        raise TypeError("boolean is not an integer receipt field")
    if isinstance(value, (str, bytes, bytearray, int)):
        return int(value)
    raise TypeError(f"unsupported integer receipt field: {type(value).__name__}")


def _easyremote_receipt_protocol_error(
    message: str, cause: BaseException | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.PROTOCOL,
        stage="easyremote_receipt",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details=profile_error_details(
            _EASYREMOTE_RECEIPT_PROFILE,
            details={"reason": "protocol"},
        ),
        cause=cause,
    )


def _easyremote_full_receipt_unavailable(invocation_id: str) -> SDKError:
    return SDKError(
        code=ErrorCode.NOT_IMPLEMENTED,
        stage="easyremote_receipt",
        retry=RetryHint.NEVER,
        retryable=False,
        message=(
            "cryptographic receipt verification needs the full receipt body, "
            "which the local summary does not provide"
        ),
        invocation_id=invocation_id or None,
        details=profile_error_details(
            _EASYREMOTE_RECEIPT_PROFILE,
            details={"reason": "full_receipt_unavailable"},
        ),
    )


def _easyremote_receipt_chain_broken(
    verification: ReceiptChainVerification,
) -> SDKError:
    broken = next((item for item in verification.items if not item.continuous), None)
    index = broken.index if broken is not None else 0
    invocation_id = broken.invocation_id if broken is not None else None
    return SDKError(
        code=ErrorCode.PROTOCOL,
        stage="easyremote_receipt",
        retry=RetryHint.NEVER,
        retryable=False,
        message=(
            f"receipt chain broken at index {index}: "
            "prev_receipt_hash does not match predecessor's self_hash"
        ),
        invocation_id=invocation_id,
        details=profile_error_details(
            _EASYREMOTE_RECEIPT_PROFILE,
            details={"reason": "receipt_chain_broken"},
        ),
    )


def _invalid_receipt(
    message: str, cause: BaseException | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="receipt",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details=profile_error_details(_PROFILE),
        cause=cause,
    )


def _transport_error(message: str, cause: BaseException) -> SDKError:
    return SDKError(
        code=ErrorCode.TRANSPORT,
        stage="transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=message,
        details=profile_error_details(_PROFILE),
        cause=cause,
    )
