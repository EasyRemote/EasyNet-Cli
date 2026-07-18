"""Runtime Core stream state facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Any, Optional, Protocol, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError


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
    selected_node_id: str = ""
    scheduling_reason: str = ""
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
        _reject_retired_top_level_receipt_alias(decoded, "stream event")
        kind = _optional_string(decoded.get("kind"), "kind")
        if not kind:
            raise _invalid_stream("stream event kind is required")
        if kind not in {"data", "terminal", "error", "cancelled", "timeout"}:
            raise _invalid_stream(f"unsupported stream event kind: {kind}")
        return cls(
            sequence=_required_positive_int(decoded, "sequence"),
            kind=kind,
            state=_optional_string(decoded.get("state"), "state") or "",
            terminal=_optional_bool(decoded.get("terminal"), "terminal") or False,
            transport_terminal=_optional_bool(
                decoded.get("transport_terminal"), "transport_terminal"
            )
            or False,
            payload_content_type=_optional_string(
                decoded.get("payload_content_type"), "payload_content_type"
            )
            or "",
            payload_base64=_optional_string(
                decoded.get("payload_base64"), "payload_base64"
            )
            or "",
            payload_json=decoded.get("payload_json"),
            selected_node_id=_optional_string(
                decoded.get("selected_node_id"), "selected_node_id"
            )
            or "",
            scheduling_reason=_optional_string(
                decoded.get("scheduling_reason"), "scheduling_reason"
            )
            or "",
            elapsed_ms=_optional_non_negative_int(
                decoded.get("elapsed_ms"), "elapsed_ms"
            ),
            error=decoded.get("error"),
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
        if self._is_terminal():
            raise _invalid_stream("stream is terminal")
        if self.state in {StreamState.TERMINAL_FRAME_SEEN, StreamState.DRAINING}:
            raise _invalid_stream("stream terminal event already seen")
        try:
            raw = self.transport.recv(timeout)
        except SDKError:
            self.state = StreamState.FAILED
            raise
        except TimeoutError:
            raise
        except Exception as exc:
            self.state = StreamState.FAILED
            raise _transport_error("stream recv transport failed", exc) from exc
        event = StreamEvent.from_json(raw)
        self._apply_event(event)
        return event

    def cancel(self, reason: str = "") -> StreamCancel:
        if self._is_terminal():
            raise _invalid_stream("stream is terminal")
        try:
            raw = self.transport.cancel(reason)
        except SDKError as exc:
            if exc.code != ErrorCode.NOT_IMPLEMENTED:
                self.state = StreamState.FAILED
            raise
        except Exception as exc:
            self.state = StreamState.FAILED
            raise _transport_error("stream cancel transport failed", exc) from exc
        outcome = StreamCancel.from_json(raw)
        if (
            outcome.state != StreamState.CANCEL_REQUESTED
            or outcome.terminal
            or outcome.cancelled
        ):
            self.state = StreamState.FAILED
            raise _invalid_stream(
                "stream cancel transport must return CancelRequested with terminal=false"
            )
        self.state = outcome.state
        return outcome

    def close(self) -> None:
        if self.state == StreamState.CLOSED:
            return
        if self.state == StreamState.TERMINAL_FRAME_SEEN:
            self.state = StreamState.DRAINING
        try:
            self.transport.close()
        except SDKError:
            self.state = StreamState.FAILED
            raise
        except Exception as exc:
            self.state = StreamState.FAILED
            raise _transport_error("stream close transport failed", exc) from exc
        self.state = StreamState.CLOSED

    def terminal_event(self) -> StreamTerminalEvent:
        if self._terminal_event is None:
            raise _invalid_stream("stream terminal event has not been seen")
        return self._terminal_event

    def _apply_event(self, event: StreamEvent) -> None:
        if event.sequence <= self._last_sequence:
            self.state = StreamState.FAILED
            raise _invalid_stream("stream events must be strictly ordered")
        if self._terminal_seen:
            self.state = StreamState.FAILED
            raise _invalid_stream("stream terminal event already seen")
        if (
            self.max_buffered_events > 0
            and len(self.events) >= self.max_buffered_events
        ):
            self.state = StreamState.FAILED
            raise _invalid_stream("stream event buffer limit exceeded")
        if self.state == StreamState.OPENING:
            self.state = StreamState.OPEN
        self._last_sequence = event.sequence
        self.events.append(event)
        if event.terminal:
            self._terminal_seen = True
            self._terminal_event = StreamTerminalEvent.from_event(self.stream_id, event)
            self.state = StreamState.TERMINAL_FRAME_SEEN
        elif event.transport_terminal:
            self.state = StreamState.FAILED

    def _is_terminal(self) -> bool:
        return self.state in {
            StreamState.CLOSED,
            StreamState.CANCELLED,
            StreamState.FAILED,
        }


def _required_positive_int(decoded: dict[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise _invalid_stream(f"{field_name} is required")
    return value


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


def _reject_retired_top_level_receipt_alias(
    decoded: dict[str, object], projection: str
) -> None:
    if "receipt" in decoded:
        raise _invalid_stream(
            f"{projection} must use terminal_receipt; retired receipt alias is not accepted"
        )


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
