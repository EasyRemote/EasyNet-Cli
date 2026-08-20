"""Runtime Core stream state facade."""

from __future__ import annotations

import json
import threading
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Any, Optional, Protocol, runtime_checkable

from ._carrier import CarrierState, is_local_carrier_interruption
from .errors import ErrorCode, RetryHint, SDKError
from ._receipt_projection import reject_retired_top_level_receipt_alias


MAX_STREAM_BUFFERED_EVENTS = 1024


class StreamState(StrEnum):
    """Runtime Core server-stream states."""

    OPENING = "Opening"
    OPEN = "Open"
    CANCEL_REQUESTED = "CancelRequested"
    TERMINAL_FRAME_SEEN = "TerminalFrameSeen"
    DRAINING = "Draining"
    CLOSED = "Closed"
    CANCELLED = "Cancelled"
    FAILED = "Failed"


@runtime_checkable
class StreamTransport(Protocol):
    """Concrete stream frame transport supplied by the integration layer."""

    def recv(self, timeout: float | None = None) -> bytes: ...

    def cancel(self, reason: str) -> bytes: ...

    def close(self) -> None: ...


@dataclass(frozen=True)
class StreamEvent:
    """SDK stream event projection."""

    sequence: int
    kind: str
    state: str = ""
    terminal: bool = False
    transport_terminal: bool = False
    payload_content_type: str = ""
    payload_base64: str = ""
    payload_json: Any = None
    elapsed_ms: int = 0
    error: Any = None
    admission_receipt: Any = None
    terminal_receipt: Any = None

    @classmethod
    def from_json(cls, raw: bytes | str) -> "StreamEvent":
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_stream(f"decode stream event JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_stream("stream event JSON must be an object")
        _reject_unknown_stream_fields(
            decoded,
            "stream event",
            "sequence",
            "kind",
            "state",
            "terminal",
            "transport_terminal",
            "payload_content_type",
            "payload_base64",
            "payload_json",
            "elapsed_ms",
            "error",
            "admission_receipt",
            "terminal_receipt",
        )
        reject_retired_top_level_receipt_alias(
            decoded, "stream event", stage="stream"
        )
        kind = _optional_string(decoded.get("kind"), "kind")
        if not kind:
            raise _invalid_stream("stream event kind is required")
        if kind not in {
            "data",
            "terminal",
            "error",
            "cancelled",
            "timeout",
            "receipt_verification_error",
        }:
            raise _invalid_stream(f"unsupported stream event kind: {kind}")
        error = decoded.get("error")
        terminal = _optional_bool(decoded.get("terminal"), "terminal") or False
        transport_terminal = (
            _optional_bool(decoded.get("transport_terminal"), "transport_terminal")
            or False
        )
        if kind == "receipt_verification_error":
            terminal = True
            transport_terminal = True
            error = error or {
                "code": "RECEIPT_VERIFICATION_FAILED",
                "stage": "receipt_verification",
                "message": _optional_string(decoded.get("message"), "message")
                or "stream receipt verification failed",
                "retryable": False,
            }
        return cls(
            sequence=_required_positive_int(decoded, "sequence"),
            kind=kind,
            state=_optional_string(decoded.get("state"), "state") or "",
            terminal=terminal,
            transport_terminal=transport_terminal,
            payload_content_type=_optional_string(
                decoded.get("payload_content_type"), "payload_content_type"
            )
            or "",
            payload_base64=_optional_string(
                decoded.get("payload_base64"), "payload_base64"
            )
            or "",
            payload_json=decoded.get("payload_json"),
            elapsed_ms=_optional_non_negative_int(
                decoded.get("elapsed_ms"), "elapsed_ms"
            ),
            error=error,
            admission_receipt=decoded.get("admission_receipt"),
            terminal_receipt=decoded.get("terminal_receipt"),
        )


@dataclass(frozen=True)
class StreamTerminalEvent:
    """Schema-shaped Runtime Core stream terminal projection."""

    stream_id: str
    event_type: str
    seq: int
    payload: Any = None
    error: Any = None
    terminal_receipt: Any = None

    @classmethod
    def from_event(cls, stream_id: str, event: StreamEvent) -> "StreamTerminalEvent":
        if not stream_id:
            raise _invalid_stream("stream_id is required")
        if not event.terminal:
            raise _invalid_stream("stream event is not terminal")
        event_type = _stream_terminal_event_type(event)
        return cls(
            stream_id=stream_id,
            event_type=event_type,
            seq=event.sequence,
            payload=event.payload_json,
            error=event.error,
            terminal_receipt=event.terminal_receipt,
        )

    def to_json(self) -> bytes:
        value: dict[str, object] = {
            "stream_id": self.stream_id,
            "event_type": self.event_type,
            "seq": self.seq,
        }
        if self.payload is not None:
            value["payload"] = self.payload
        if self.error is not None:
            value["error"] = self.error
        if self.terminal_receipt is not None:
            value["terminal_receipt"] = self.terminal_receipt
        return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


@dataclass(frozen=True)
class StreamCancel:
    """Stream cancellation outcome projection."""

    stream_id: str
    cancelled: bool
    state: StreamState
    terminal: bool

    @classmethod
    def from_json(cls, raw: bytes | str) -> "StreamCancel":
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_stream(f"decode stream cancel JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_stream("stream cancel JSON must be an object")
        _reject_unknown_stream_fields(
            decoded,
            "stream cancel",
            "stream_id",
            "cancelled",
            "state",
            "terminal",
        )
        state = _stream_state(_required_string(decoded, "state"))
        if state not in {
            StreamState.CANCEL_REQUESTED,
            StreamState.CANCELLED,
            StreamState.CLOSED,
            StreamState.FAILED,
        }:
            raise _invalid_stream(
                "stream cancel state must be CancelRequested, Cancelled, Closed, or Failed"
            )
        terminal = _required_bool(decoded, "terminal")
        if state == StreamState.CANCEL_REQUESTED and terminal:
            raise _invalid_stream("stream cancel request must not be terminal")
        return cls(
            stream_id=_required_string(decoded, "stream_id"),
            cancelled=_required_bool(decoded, "cancelled"),
            state=state,
            terminal=terminal,
        )


@dataclass
class StreamHandle:
    """Ordered stream event state object."""

    stream_id: str
    transport: StreamTransport
    state: StreamState = StreamState.OPENING
    max_buffered_events: int = MAX_STREAM_BUFFERED_EVENTS
    events: list[StreamEvent] = field(default_factory=list)
    _last_sequence: int = 0
    _terminal_seen: bool = False
    _terminal_event: Optional[StreamTerminalEvent] = None
    _runtime_state: StreamState = field(init=False, repr=False)
    _carrier_state: CarrierState = field(
        default=CarrierState.OPEN, init=False, repr=False
    )
    _lock: Any = field(
        default_factory=threading.RLock, init=False, repr=False, compare=False
    )
    _receiving: bool = field(default=False, init=False, repr=False)

    def __post_init__(self) -> None:
        self._runtime_state = self.state

    @property
    def runtime_state(self) -> StreamState:
        """Provider-observed lifecycle state, excluding local carrier close."""

        with self._lock:
            return self._runtime_state

    @classmethod
    def from_json(cls, transport: StreamTransport, raw: bytes | str) -> "StreamHandle":
        if transport is None:
            raise _invalid_stream("stream transport is required")
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text) if text else {}
        except Exception as exc:
            raise _invalid_stream(f"decode stream open JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_stream("stream open JSON must be an object")
        _reject_unknown_stream_fields(
            decoded,
            "stream open",
            "stream_id",
            "state",
            "max_buffered_events",
        )
        state = _stream_state(
            _optional_string(decoded.get("state"), "state") or "Opening"
        )
        if state not in {StreamState.OPENING, StreamState.OPEN}:
            raise _invalid_stream("stream open state must be Opening or Open")
        max_buffered = _optional_non_negative_int(
            decoded.get("max_buffered_events"), "max_buffered_events"
        )
        if max_buffered == 0:
            max_buffered = MAX_STREAM_BUFFERED_EVENTS
        return cls(
            stream_id=_required_string(decoded, "stream_id"),
            transport=transport,
            state=state,
            max_buffered_events=max_buffered,
        )

    def next(self, timeout: float | None = None) -> StreamEvent:
        with self._lock:
            self._require_carrier_open_locked()
            if self._is_runtime_terminal_locked():
                raise _invalid_stream("stream is terminal")
            if self._runtime_state in {
                StreamState.TERMINAL_FRAME_SEEN,
                StreamState.DRAINING,
            }:
                raise _invalid_stream("stream terminal event already seen")
            if self._receiving:
                raise _invalid_stream("stream recv is already in progress")
            self._receiving = True
            transport = self.transport
        try:
            raw = transport.recv(timeout)
        except SDKError as exc:
            with self._lock:
                self._receiving = False
                if self._carrier_state.is_open and not is_local_carrier_interruption(
                    exc
                ):
                    self._set_runtime_state_locked(StreamState.FAILED)
            raise
        except TimeoutError:
            with self._lock:
                self._receiving = False
            raise
        except Exception as exc:
            with self._lock:
                self._receiving = False
                if self._carrier_state.is_open:
                    self._set_runtime_state_locked(StreamState.FAILED)
            raise _transport_error("stream recv transport failed", exc) from exc
        try:
            event = StreamEvent.from_json(raw)
        except SDKError:
            with self._lock:
                self._receiving = False
                self._set_runtime_state_locked(StreamState.FAILED)
            raise
        with self._lock:
            self._receiving = False
            if self._is_runtime_terminal_locked() and not event.terminal:
                raise _invalid_stream(
                    "stream became terminal while receive was in progress"
                )
            self._apply_event_locked(event)
        return event

    def cancel(self, reason: str = "") -> StreamCancel:
        with self._lock:
            self._require_carrier_open_locked()
            if self._is_runtime_terminal_locked() or self._runtime_state in {
                StreamState.TERMINAL_FRAME_SEEN,
                StreamState.DRAINING,
            }:
                raise _invalid_stream("stream is terminal")
            transport = self.transport
        try:
            raw = transport.cancel(reason)
        except SDKError as exc:
            if (
                exc.code != ErrorCode.NOT_IMPLEMENTED
                and not is_local_carrier_interruption(exc)
            ):
                with self._lock:
                    self._set_runtime_state_locked(StreamState.FAILED)
            raise
        except Exception as exc:
            if not is_local_carrier_interruption(exc):
                with self._lock:
                    self._set_runtime_state_locked(StreamState.FAILED)
            raise _transport_error("stream cancel transport failed", exc) from exc
        try:
            outcome = StreamCancel.from_json(raw)
        except SDKError:
            with self._lock:
                self._set_runtime_state_locked(StreamState.FAILED)
            raise
        if (
            outcome.state != StreamState.CANCEL_REQUESTED
            or outcome.terminal
            or outcome.cancelled
        ):
            with self._lock:
                self._set_runtime_state_locked(StreamState.FAILED)
            raise _invalid_stream(
                "stream cancel transport must return CancelRequested with terminal=false"
            )
        with self._lock:
            if self._runtime_state in {
                StreamState.TERMINAL_FRAME_SEEN,
                StreamState.DRAINING,
            }:
                return outcome
            if self._runtime_state == StreamState.FAILED:
                raise _invalid_stream("stream failed while cancellation was in flight")
            self._set_runtime_state_locked(outcome.state)
        return outcome

    def close(self) -> None:
        with self._lock:
            if self._carrier_state is CarrierState.CLOSED:
                return
            if self._carrier_state is CarrierState.CLOSING:
                raise _invalid_stream("stream carrier close is already in progress")
            self._carrier_state = CarrierState.CLOSING
            transport = self.transport
        try:
            transport.close()
        except SDKError:
            with self._lock:
                self._carrier_state = CarrierState.FAILED
                self.state = StreamState.FAILED
            raise
        except Exception as exc:
            with self._lock:
                self._carrier_state = CarrierState.FAILED
                self.state = StreamState.FAILED
            raise _transport_error("stream close transport failed", exc) from exc
        with self._lock:
            self._carrier_state = CarrierState.CLOSED
            self.state = StreamState.CLOSED

    def terminal_event(self) -> StreamTerminalEvent:
        with self._lock:
            if self._terminal_event is None:
                raise _invalid_stream("stream terminal event has not been seen")
            return self._terminal_event

    def _apply_event_locked(self, event: StreamEvent) -> None:
        if event.sequence <= self._last_sequence:
            self._set_runtime_state_locked(StreamState.FAILED)
            raise _invalid_stream("stream events must be strictly ordered")
        if self._terminal_seen:
            self._set_runtime_state_locked(StreamState.FAILED)
            raise _invalid_stream("stream terminal event already seen")
        if (
            self.max_buffered_events > 0
            and len(self.events) >= self.max_buffered_events
        ):
            self._set_runtime_state_locked(StreamState.FAILED)
            raise _invalid_stream("stream event buffer limit exceeded")
        if self._runtime_state == StreamState.OPENING:
            self._set_runtime_state_locked(StreamState.OPEN)
        self._last_sequence = event.sequence
        self.events.append(event)
        if event.transport_terminal:
            if event.terminal:
                self._terminal_seen = True
                self._terminal_event = StreamTerminalEvent.from_event(
                    self.stream_id, event
                )
            self._carrier_state = CarrierState.FAILED
            self.state = StreamState.FAILED
        elif event.terminal:
            self._terminal_seen = True
            self._terminal_event = StreamTerminalEvent.from_event(self.stream_id, event)
            self._set_runtime_state_locked(StreamState.TERMINAL_FRAME_SEEN)

    def _set_runtime_state_locked(self, state: StreamState) -> None:
        self._runtime_state = state
        if self._carrier_state.is_open:
            self.state = state

    def _require_carrier_open_locked(self) -> None:
        if not self._carrier_state.is_open:
            raise _invalid_stream("stream carrier is closed")

    def _is_runtime_terminal_locked(self) -> bool:
        return self._runtime_state in {
            StreamState.CANCELLED,
            StreamState.FAILED,
        }


def _required_positive_int(decoded: dict[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise _invalid_stream(f"{field_name} is required")
    return value


def _reject_unknown_stream_fields(
    decoded: dict[str, object], projection: str, *allowed_fields: str
) -> None:
    allowed = set(allowed_fields)
    for field_name in decoded:
        if field_name not in allowed:
            raise _invalid_stream(
                f"{projection} contains noncanonical field {field_name}"
            )


def _stream_state(value: str) -> StreamState:
    try:
        return StreamState(value)
    except ValueError as exc:
        raise _invalid_stream(f"unknown stream state: {value}", exc) from exc


def _optional_non_negative_int(value: object, field_name: str) -> int:
    if value is None:
        return 0
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_stream(f"{field_name} must be a non-negative integer")
    return value


def _required_string(decoded: dict[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_stream(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_stream(f"{field_name} must be a string or null")
    return value


def _required_bool(decoded: dict[str, object], field_name: str) -> bool:
    value = decoded.get(field_name)
    if not isinstance(value, bool):
        raise _invalid_stream(f"{field_name} must be a boolean")
    return value


def _optional_bool(value: object, field_name: str) -> Optional[bool]:
    if value is None:
        return None
    if not isinstance(value, bool):
        raise _invalid_stream(f"{field_name} must be a boolean or null")
    return value


def _stream_terminal_event_type(event: StreamEvent) -> str:
    if event.kind in {"terminal", "error", "cancelled", "timeout"}:
        return event.kind
    if event.error is not None:
        return "error"
    return "terminal"


def _invalid_stream(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="stream",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )


def _transport_error(message: str, cause: BaseException) -> SDKError:
    return SDKError(
        code=ErrorCode.ROUTE_UNAVAILABLE,
        stage="transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=message,
        cause=cause,
    )
