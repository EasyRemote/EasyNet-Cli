"""Runtime Core prepare and submit facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Mapping, Optional, Protocol, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError
from .bidi import BidiSession, BidiStreamDescriptor, BidiTransport
from .invocation import InvocationBuilder, InvocationDraft
from .stream import StreamHandle, StreamTransport
from .signing import PreparedInvocation, SignedInvocation, Signer, SigningMaterial


@runtime_checkable
class RuntimeTransport(Protocol):
    """Narrow transport seam owned by the application integration layer."""

    def invoke(self, draft_json: bytes) -> bytes:
        ...

    def open_stream(self, draft_json: bytes) -> tuple[StreamTransport, bytes]:
        ...

    def open_bidi(
        self, draft_json: bytes, streams_json: bytes
    ) -> tuple[BidiTransport, bytes]:
        ...

    def prepare(self, draft_json: bytes, options_json: bytes) -> bytes:
        ...

    def submit_signed(self, signed_json: bytes) -> bytes:
        ...

    def await_handle(self, handle_id: int) -> bytes:
        ...

    def cancel_handle(self, handle_id: int, reason: str) -> bytes:
        ...

    def handle_events(self, handle_id: int) -> bytes:
        ...

    def free_handle(self, handle_id: int) -> None:
        ...

    def close(self) -> None:
        ...


@dataclass(frozen=True)
class PrepareOptions:
    """Daemon-owned prepare policy knobs."""

    resolve_descriptor: bool = False
    fill_nonce: bool = False
    require_user_sig: bool = False
    expires_in_ms: int = 0

    def to_json_dict(self) -> dict[str, object]:
        value: dict[str, object] = {}
        if self.resolve_descriptor:
            value["resolve_descriptor"] = self.resolve_descriptor
        if self.fill_nonce:
            value["fill_nonce"] = self.fill_nonce
        if self.require_user_sig:
            value["require_user_sig"] = self.require_user_sig
        if self.expires_in_ms:
            value["expires_in_ms"] = self.expires_in_ms
        return value

    def to_json_bytes(self) -> bytes:
        return json.dumps(
            self.to_json_dict(), separators=(",", ":"), sort_keys=True
        ).encode("utf-8")


@dataclass(frozen=True)
class InvocationHandleEvent:
    """Submitted invocation event projection."""

    sequence: int
    kind: str
    state: str
    terminal: bool
    reason: Optional[str] = None
    result: Optional[Mapping[str, object]] = None


@dataclass(frozen=True)
class InvocationHandle:
    """Submitted invocation observation handle projection."""

    handle_id: int
    state: str
    terminal: bool
    events: tuple[InvocationHandleEvent, ...] = field(default_factory=tuple)
    result: Optional[Mapping[str, object]] = None

    @classmethod
    def from_json(cls, raw: bytes | str) -> "InvocationHandle":
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_runtime(f"decode invocation handle JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_runtime("invocation handle JSON must be an object")

        handle_id = _required_positive_int(decoded, "handle_id")
        state = _required_string(decoded, "state")
        terminal = _required_bool(decoded, "terminal")
        raw_events = decoded.get("events", [])
        if not isinstance(raw_events, list):
            raise _invalid_runtime("events must be an array")
        events = tuple(_handle_event(item) for item in raw_events)
        result = _optional_mapping(decoded.get("result"), "result")
        return cls(
            handle_id=handle_id,
            state=state,
            terminal=terminal,
            events=events,
            result=result,
        )


@dataclass(frozen=True)
class InvocationFailure:
    """Runtime failure embedded in a terminal invocation result."""

    code: str
    stage: str
    message: str = ""
    retryable: bool = False


@dataclass(frozen=True)
class RuntimeReceipt:
    """Non-verifying Runtime Core terminal receipt projection."""

    raw: Mapping[str, object]
    receipt_id: str = ""
    receipt_ura: str = ""
    invocation_id: str = ""
    receipt_type: str = ""
    state: str = ""
    index: int = 0
    timestamp_unix_ms: int = 0
    prev_receipt_hash_hex: str = ""
    self_hash_hex: str = ""
    cleanup_complete: Optional[bool] = None
    reason: str = ""
    child_invocation_id: str = ""

    @classmethod
    def from_mapping(cls, decoded: Mapping[str, object]) -> "RuntimeReceipt":
        return cls(
            raw=dict(decoded),
            receipt_id=_optional_string(decoded.get("receipt_id"), "receipt_id") or "",
            receipt_ura=_optional_string(decoded.get("receipt_ura"), "receipt_ura") or "",
            invocation_id=_optional_string(decoded.get("invocation_id"), "invocation_id")
            or "",
            receipt_type=_optional_string(decoded.get("receipt_type"), "receipt_type")
            or "",
            state=_optional_string(decoded.get("state"), "state") or "",
            index=_optional_non_negative_int(decoded.get("index"), "index"),
            timestamp_unix_ms=_optional_non_negative_int(
                decoded.get("timestamp_unix_ms"), "timestamp_unix_ms"
            ),
            prev_receipt_hash_hex=_optional_string(
                decoded.get("prev_receipt_hash_hex"), "prev_receipt_hash_hex"
            )
            or "",
            self_hash_hex=_optional_string(decoded.get("self_hash_hex"), "self_hash_hex")
            or "",
            cleanup_complete=_optional_bool(
                decoded.get("cleanup_complete"), "cleanup_complete"
            ),
            reason=_optional_string(decoded.get("reason"), "reason") or "",
            child_invocation_id=_optional_string(
                decoded.get("child_invocation_id"), "child_invocation_id"
            )
            or "",
        )

    def has_causal_anchor(self) -> bool:
        """Return whether daemon/Axon supplied enough facts for causal linkage."""

        return bool(self.receipt_ura and self.self_hash_hex)

    def to_json_dict(self) -> dict[str, object]:
        return dict(self.raw)


@dataclass(frozen=True)
class InvocationResult:
    """Unary invocation terminal result projection."""

    ok: bool
    tuple: InvocationDraft
    terminal_state: str
    output_content_type: str = ""
    output_base64: str = ""
    output_json: Any = None
    selected_node_id: str = ""
    scheduling_reason: str = ""
    elapsed_ms: int = 0
    receipt: Optional[Mapping[str, object]] = None
    receipt_summary: Optional[RuntimeReceipt] = None
    error: Optional[InvocationFailure] = None

    @classmethod
    def from_json(cls, raw: bytes | str) -> "InvocationResult":
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_runtime(f"decode invocation result JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_runtime("invocation result JSON must be an object")
        ok = _required_bool(decoded, "ok")
        tuple_value = _required_mapping(decoded, "tuple")
        draft = InvocationDraft.from_json(json.dumps(tuple_value))
        terminal_state = _required_string(decoded, "terminal_state")
        elapsed_ms = _optional_non_negative_int(decoded.get("elapsed_ms"), "elapsed_ms")
        failure = _failure(decoded.get("error"))
        if ok and failure is not None:
            raise _invalid_runtime("ok result must not include error")
        if not ok and failure is None:
            raise _invalid_runtime("failed result must include error")
        receipt = _optional_mapping(decoded.get("receipt"), "receipt")
        receipt_summary = RuntimeReceipt.from_mapping(receipt) if receipt else None
        return cls(
            ok=ok,
            tuple=draft,
            terminal_state=terminal_state,
            output_content_type=_optional_string(
                decoded.get("output_content_type"), "output_content_type"
            )
            or "",
            output_base64=_optional_string(decoded.get("output_base64"), "output_base64")
            or "",
            output_json=decoded.get("output_json"),
            selected_node_id=_optional_string(
                decoded.get("selected_node_id"), "selected_node_id"
            )
            or "",
            scheduling_reason=_optional_string(
                decoded.get("scheduling_reason"), "scheduling_reason"
            )
            or "",
            elapsed_ms=elapsed_ms,
            receipt=receipt,
            receipt_summary=receipt_summary,
            error=failure,
        )


@dataclass(frozen=True)
class InvocationCancel:
    """Daemon cancellation outcome for a submitted handle."""

    handle_id: int
    cancelled: bool
    state: str
    terminal: bool

    @classmethod
    def from_json(cls, raw: bytes | str) -> "InvocationCancel":
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_runtime(f"decode invocation cancel JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_runtime("invocation cancel JSON must be an object")
        return cls(
            handle_id=_required_positive_int(decoded, "handle_id"),
            cancelled=_required_bool(decoded, "cancelled"),
            state=_required_string(decoded, "state"),
            terminal=_required_bool(decoded, "terminal"),
        )


class RuntimeClient:
    """Runtime Core invocation facade over an application transport."""

    def __init__(self, transport: RuntimeTransport) -> None:
        if transport is None:
            raise _invalid_runtime_client("runtime transport is required")
        self._transport = transport
        self._closed = False

    def invoke(self, draft: InvocationDraft) -> InvocationResult:
        transport = self._require_open()
        try:
            raw = transport.invoke(draft.to_json().encode("utf-8"))
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("invoke transport failed", exc) from exc
        return InvocationResult.from_json(raw)

    def invoke_stream(self, draft: InvocationDraft) -> StreamHandle:
        transport = self._require_open()
        try:
            stream_transport, open_json = transport.open_stream(
                draft.to_json().encode("utf-8")
            )
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("open stream transport failed", exc) from exc
        return StreamHandle.from_json(stream_transport, open_json)

    def open_bidi(
        self,
        draft: InvocationDraft,
        streams: tuple[BidiStreamDescriptor, ...],
    ) -> BidiSession:
        transport = self._require_open()
        try:
            streams_json = json.dumps(
                [stream.to_json_dict() for stream in streams],
                separators=(",", ":"),
                sort_keys=True,
            ).encode("utf-8")
            bidi_transport, open_json = transport.open_bidi(
                draft.to_json().encode("utf-8"),
                streams_json,
            )
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("open bidi transport failed", exc) from exc
        return BidiSession.from_json(bidi_transport, open_json)

    def prepare(
        self,
        draft: InvocationDraft,
        options: PrepareOptions = PrepareOptions(),
    ) -> tuple[PreparedInvocation, SigningMaterial]:
        transport = self._require_open()
        try:
            draft_json = draft.to_json().encode("utf-8")
            options_json = options.to_json_bytes()
            raw = transport.prepare(draft_json, options_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("prepare transport failed", exc) from exc
        prepared = PreparedInvocation.from_json(raw)
        return prepared, prepared.signing_material

    def prepare_builder(
        self,
        builder: InvocationBuilder,
        options: PrepareOptions = PrepareOptions(),
    ) -> tuple[PreparedInvocation, SigningMaterial]:
        """Prepare a builder and consume it only after prepare succeeds."""

        if builder is None:
            raise _invalid_runtime("invocation builder is required")
        draft = builder.inspect()
        prepared, material = self.prepare(draft, options)
        builder._consume()
        return prepared, material

    def prepare_and_sign(
        self,
        draft: InvocationDraft,
        signer: Signer,
        options: PrepareOptions = PrepareOptions(),
    ) -> tuple[SignedInvocation, SigningMaterial]:
        """Prepare canonical material and return an inspectable signed envelope."""

        if signer is None:
            raise _invalid_runtime("signer is required")
        prepared, material = self.prepare(draft, options)
        return signer.sign(prepared), material

    def submit_signed(self, signed: SignedInvocation) -> InvocationHandle:
        transport = self._require_open()
        if not signed.submit_ready():
            raise _invalid_runtime("signed invocation is not submit-ready")
        try:
            raw = transport.submit_signed(signed.to_json().encode("utf-8"))
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("submit signed transport failed", exc) from exc
        return InvocationHandle.from_json(raw)

    def await_result(self, handle: InvocationHandle) -> InvocationResult:
        transport = self._require_open()
        _require_handle(handle)
        try:
            raw = transport.await_handle(handle.handle_id)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("await handle transport failed", exc) from exc
        return InvocationResult.from_json(raw)

    def cancel(self, handle: InvocationHandle, reason: str = "") -> InvocationCancel:
        transport = self._require_open()
        _require_handle(handle)
        try:
            raw = transport.cancel_handle(handle.handle_id, reason)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("cancel handle transport failed", exc) from exc
        return InvocationCancel.from_json(raw)

    def events(self, handle: InvocationHandle) -> InvocationHandle:
        transport = self._require_open()
        _require_handle(handle)
        try:
            raw = transport.handle_events(handle.handle_id)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("handle events transport failed", exc) from exc
        return InvocationHandle.from_json(raw)

    def close_handle(self, handle: InvocationHandle) -> None:
        transport = self._require_open()
        _require_handle(handle)
        try:
            transport.free_handle(handle.handle_id)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("free handle transport failed", exc) from exc

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            self._transport.close()
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("runtime close transport failed", exc) from exc

    def _require_open(self) -> RuntimeTransport:
        if self._closed:
            raise _invalid_runtime_client("runtime client is closed")
        return self._transport


def _require_handle(handle: InvocationHandle) -> None:
    if handle.handle_id <= 0:
        raise _invalid_runtime("handle_id is required")


def _handle_event(value: object) -> InvocationHandleEvent:
    if not isinstance(value, dict):
        raise _invalid_runtime("event must be an object")
    return InvocationHandleEvent(
        sequence=_required_positive_int(value, "sequence"),
        kind=_required_string(value, "kind"),
        state=_required_string(value, "state"),
        terminal=_required_bool(value, "terminal"),
        reason=_optional_string(value.get("reason"), "reason"),
        result=_optional_mapping(value.get("result"), "result"),
    )


def _failure(value: object) -> Optional[InvocationFailure]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_runtime("error must be an object or null")
    return InvocationFailure(
        code=_required_string(value, "code"),
        stage=_required_string(value, "stage"),
        message=_optional_string(value.get("message"), "message") or "",
        retryable=_optional_bool(value.get("retryable"), "retryable") or False,
    )


def _required_mapping(
    decoded: Mapping[str, object], field_name: str
) -> Mapping[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise _invalid_runtime(f"{field_name} must be an object")
    return value


def _required_positive_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise _invalid_runtime(f"{field_name} is required")
    return value


def _optional_non_negative_int(value: object, field_name: str) -> int:
    if value is None:
        return 0
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_runtime(f"{field_name} must be a non-negative integer")
    return value


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_runtime(f"{field_name} is required")
    return value


def _required_bool(decoded: Mapping[str, object], field_name: str) -> bool:
    value = decoded.get(field_name)
    if not isinstance(value, bool):
        raise _invalid_runtime(f"{field_name} must be a boolean")
    return value


def _optional_bool(value: object, field_name: str) -> Optional[bool]:
    if value is None:
        return None
    if not isinstance(value, bool):
        raise _invalid_runtime(f"{field_name} must be a boolean or null")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_runtime(f"{field_name} must be a string or null")
    return value


def _optional_mapping(
    value: object, field_name: str
) -> Optional[Mapping[str, object]]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_runtime(f"{field_name} must be an object or null")
    return dict(value)


def _invalid_runtime_client(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="sdk",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )


def _invalid_runtime(
    message: str, cause: Optional[BaseException] = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime",
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
