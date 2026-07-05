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
    payload_content_type: str = ""
    payload_base64: str = ""
    payload_json: Any = None
    error: Any = None

    @classmethod
    def from_json(cls, raw: bytes | str) -> "StreamEvent":
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_stream(f"decode stream event JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_stream("stream event JSON must be an object")
        kind = _optional_string(decoded.get("kind"), "kind") or _optional_string(
            decoded.get("event"), "event"
        )
        if not kind:
            raise _invalid_stream("stream event kind is required")
        return cls(
            sequence=_required_positive_int(decoded, "sequence"),
            kind=kind,
            state=_optional_string(decoded.get("state"), "state") or "",
            terminal=_optional_bool(decoded.get("terminal"), "terminal") or False,
            payload_content_type=_optional_string(
                decoded.get("payload_content_type"), "payload_content_type"
            )
            or _optional_string(decoded.get("content_type"), "content_type")
            or "",
            payload_base64=_optional_string(
                decoded.get("payload_base64"), "payload_base64"
            )
            or "",
            payload_json=decoded.get("payload_json"),
            error=decoded.get("error"),
        )


@dataclass(frozen=True)
class StreamTerminalEvent:
    """Schema-shaped Runtime Core stream terminal projection."""

    stream_id: str
    event_type: str
    seq: int
    payload: Any = None
    error: Any = None
    receipt: Any = None

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
            receipt=_receipt_from_payload(event.payload_json),
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
        if self.receipt is not None:
            value["receipt"] = self.receipt
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
        if state not in {StreamState.CANCELLED, StreamState.CLOSED, StreamState.FAILED}:
            raise _invalid_stream(
                "stream cancel state must be Cancelled, Closed, or Failed"
            )
        return cls(
            stream_id=_required_string(decoded, "stream_id"),
            cancelled=_required_bool(decoded, "cancelled"),
            state=state,
            terminal=_required_bool(decoded, "terminal"),
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
        except SDKError:
            self.state = StreamState.FAILED
            raise
        except Exception as exc:
            self.state = StreamState.FAILED
            raise _transport_error("stream cancel transport failed", exc) from exc
        outcome = StreamCancel.from_json(raw)
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


def _receipt_from_payload(payload: Any) -> Any:
    if isinstance(payload, dict) and isinstance(payload.get("receipt"), dict):
        return payload["receipt"]
    return None


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
        code=ErrorCode.TRANSPORT,
        stage="transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=message,
        cause=cause,
    )
