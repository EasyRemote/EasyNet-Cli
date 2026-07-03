"""Runtime Core bidirectional session state facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Any, Optional, Protocol, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError


MAX_BIDI_BUFFERED_FRAMES = 1024


class BidiState(StrEnum):
    """Runtime Core bidirectional session states."""

    CREATED = "Created"
    OPENING = "Opening"
    OPEN = "Open"
    HALF_CLOSED_LOCAL = "HalfClosedLocal"
    HALF_CLOSED_REMOTE = "HalfClosedRemote"
    TERMINAL = "Terminal"
    CLOSED = "Closed"
    CANCELLED = "Cancelled"
    FAILED = "Failed"


@dataclass(frozen=True)
class BidiStreamDescriptor:
    """Logical stream descriptor requested for a bidi session."""

    stream_id: int
    content_type: str = ""
    codec_params: str = ""
    ordering: str = ""

    def to_json_dict(self) -> dict[str, object]:
        value: dict[str, object] = {"stream_id": self.stream_id}
        if self.content_type:
            value["content_type"] = self.content_type
        if self.codec_params:
            value["codec_params"] = self.codec_params
        if self.ordering:
            value["ordering"] = self.ordering
        return value


@runtime_checkable
class BidiTransport(Protocol):
    """Concrete bidi frame transport supplied by the integration layer."""

    def send(self, frame_json: bytes) -> bytes:
        ...

    def recv(self) -> bytes:
        ...

    def close_send(self) -> bytes:
        ...

    def close(self) -> None:
        ...

    def cancel(self, reason: str) -> bytes:
        ...


@dataclass(frozen=True)
class BidiFrame:
    """SDK bidi frame projection."""

    sequence: int
    kind: str
    stream_id: int = 0
    terminal: bool = False
    payload_content_type: str = ""
    payload_base64: str = ""
    payload_json: Any = None
    error: Any = None

    @classmethod
    def from_json(cls, raw: bytes | str) -> "BidiFrame":
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_bidi(f"decode bidi frame JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_bidi("bidi frame JSON must be an object")
        kind = _optional_string(decoded.get("kind"), "kind") or _optional_string(
            decoded.get("event"), "event"
        )
        if not kind:
            raise _invalid_bidi("bidi frame kind is required")
        return cls(
            sequence=_required_positive_int(decoded, "sequence"),
            kind=kind,
            stream_id=_optional_non_negative_int(decoded.get("stream_id"), "stream_id"),
            terminal=_optional_bool(decoded.get("terminal"), "terminal") or False,
            payload_content_type=_optional_string(
                decoded.get("payload_content_type"), "payload_content_type"
            )
            or "",
            payload_base64=_optional_string(decoded.get("payload_base64"), "payload_base64")
            or "",
            payload_json=decoded.get("payload_json"),
            error=decoded.get("error"),
        )

    def to_json(self) -> bytes:
        value: dict[str, object] = {
            "sequence": self.sequence,
            "kind": self.kind,
            "stream_id": self.stream_id,
            "terminal": self.terminal,
        }
        if self.payload_content_type:
            value["payload_content_type"] = self.payload_content_type
        if self.payload_base64:
            value["payload_base64"] = self.payload_base64
        if self.payload_json is not None:
            value["payload_json"] = self.payload_json
        if self.error is not None:
            value["error"] = self.error
        return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


@dataclass(frozen=True)
class BidiOutcome:
    """Bidi lifecycle outcome projection."""

    session_id: str
    state: BidiState
    terminal: bool
    reason: str = ""

    @classmethod
    def from_json(cls, raw: bytes | str) -> "BidiOutcome":
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_bidi(f"decode bidi outcome JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_bidi("bidi outcome JSON must be an object")
        state = _bidi_state(_required_string(decoded, "state"))
        if state not in {
            BidiState.HALF_CLOSED_LOCAL,
            BidiState.HALF_CLOSED_REMOTE,
            BidiState.TERMINAL,
            BidiState.CANCELLED,
            BidiState.CLOSED,
            BidiState.FAILED,
        }:
            raise _invalid_bidi("invalid bidi outcome state")
        terminal = _required_bool(decoded, "terminal")
        if state in {
            BidiState.TERMINAL,
            BidiState.CANCELLED,
            BidiState.CLOSED,
            BidiState.FAILED,
        } and not terminal:
            raise _invalid_bidi("terminal bidi outcome must set terminal")
        return cls(
            session_id=_required_string(decoded, "session_id"),
            state=state,
            terminal=terminal,
            reason=_optional_string(decoded.get("reason"), "reason") or "",
        )


@dataclass
class BidiSession:
    """Bidirectional session lifecycle object."""

    session_id: str
    transport: BidiTransport
    state: BidiState = BidiState.OPENING
    max_buffered_frames: int = MAX_BIDI_BUFFERED_FRAMES
    sent_frames: list[BidiFrame] = field(default_factory=list)
    received_frames: list[BidiFrame] = field(default_factory=list)
    _last_send_sequence: int = 0
    _last_recv_sequence: int = 0

    @classmethod
    def from_json(cls, transport: BidiTransport, raw: bytes | str) -> "BidiSession":
        if transport is None:
            raise _invalid_bidi("bidi transport is required")
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_bidi(f"decode bidi open JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_bidi("bidi open JSON must be an object")
        state = _bidi_state(_optional_string(decoded.get("state"), "state") or "Opening")
        if state not in {BidiState.OPENING, BidiState.OPEN}:
            raise _invalid_bidi("bidi open state must be Opening or Open")
        max_buffered = _optional_non_negative_int(
            decoded.get("max_buffered_frames"), "max_buffered_frames"
        )
        if max_buffered == 0:
            max_buffered = MAX_BIDI_BUFFERED_FRAMES
        return cls(
            session_id=_required_string(decoded, "session_id"),
            transport=transport,
            state=state,
            max_buffered_frames=max_buffered,
        )

    def send(self, frame: BidiFrame) -> BidiFrame:
        if self.state != BidiState.OPEN:
            raise _invalid_bidi("bidi send path is closed")
        if self.max_buffered_frames > 0 and len(self.sent_frames) >= self.max_buffered_frames:
            self.state = BidiState.FAILED
            raise _invalid_bidi("bidi send buffer limit exceeded")
        try:
            raw = self.transport.send(frame.to_json())
        except SDKError:
            self.state = BidiState.FAILED
            raise
        except Exception as exc:
            self.state = BidiState.FAILED
            raise _transport_error("bidi send transport failed", exc) from exc
        ack = BidiFrame.from_json(raw)
        self._record_sent(ack)
        return ack

    def receive(self) -> BidiFrame:
        if self._is_terminal() or self.state == BidiState.TERMINAL:
            raise _invalid_bidi("bidi session is terminal")
        try:
            raw = self.transport.recv()
        except SDKError:
            self.state = BidiState.FAILED
            raise
        except Exception as exc:
            self.state = BidiState.FAILED
            raise _transport_error("bidi recv transport failed", exc) from exc
        frame = BidiFrame.from_json(raw)
        self._record_received(frame)
        self._apply_received_state(frame)
        return frame

    def close_send(self) -> BidiOutcome:
        if self.state not in {BidiState.OPEN, BidiState.HALF_CLOSED_REMOTE}:
            raise _invalid_bidi("bidi send path is closed")
        try:
            raw = self.transport.close_send()
        except SDKError:
            self.state = BidiState.FAILED
            raise
        except Exception as exc:
            self.state = BidiState.FAILED
            raise _transport_error("bidi close-send transport failed", exc) from exc
        outcome = BidiOutcome.from_json(raw)
        if self.state == BidiState.HALF_CLOSED_REMOTE:
            self.state = BidiState.TERMINAL
            return BidiOutcome(
                session_id=outcome.session_id,
                state=BidiState.TERMINAL,
                terminal=True,
                reason=outcome.reason,
            )
        else:
            self.state = outcome.state
        return outcome

    def cancel(self, reason: str = "") -> BidiOutcome:
        if self._is_terminal():
            raise _invalid_bidi("bidi session is terminal")
        try:
            raw = self.transport.cancel(reason)
        except SDKError:
            self.state = BidiState.FAILED
            raise
        except Exception as exc:
            self.state = BidiState.FAILED
            raise _transport_error("bidi cancel transport failed", exc) from exc
        outcome = BidiOutcome.from_json(raw)
        self.state = outcome.state
        return outcome

    def close(self) -> None:
        if self.state == BidiState.CLOSED:
            return
        if self.state not in {BidiState.TERMINAL, BidiState.CANCELLED, BidiState.FAILED}:
            raise _invalid_bidi("bidi session must be terminal before close")
        try:
            self.transport.close()
        except SDKError:
            self.state = BidiState.FAILED
            raise
        except Exception as exc:
            self.state = BidiState.FAILED
            raise _transport_error("bidi close transport failed", exc) from exc
        self.state = BidiState.CLOSED

    def _record_sent(self, frame: BidiFrame) -> None:
        if frame.sequence <= self._last_send_sequence:
            self.state = BidiState.FAILED
            raise _invalid_bidi("bidi sent frames must be strictly ordered")
        self._last_send_sequence = frame.sequence
        self.sent_frames.append(frame)

    def _record_received(self, frame: BidiFrame) -> None:
        if frame.sequence <= self._last_recv_sequence:
            self.state = BidiState.FAILED
            raise _invalid_bidi("bidi received frames must be strictly ordered")
        if self.max_buffered_frames > 0 and len(self.received_frames) >= self.max_buffered_frames:
            self.state = BidiState.FAILED
            raise _invalid_bidi("bidi receive buffer limit exceeded")
        self._last_recv_sequence = frame.sequence
        self.received_frames.append(frame)

    def _apply_received_state(self, frame: BidiFrame) -> None:
        if frame.terminal:
            self.state = BidiState.TERMINAL
        elif frame.kind == "remote_close_send":
            if self.state == BidiState.HALF_CLOSED_LOCAL:
                self.state = BidiState.TERMINAL
            else:
                self.state = BidiState.HALF_CLOSED_REMOTE
        elif self.state == BidiState.OPENING:
            self.state = BidiState.OPEN

    def _is_terminal(self) -> bool:
        return self.state in {
            BidiState.TERMINAL,
            BidiState.CLOSED,
            BidiState.CANCELLED,
            BidiState.FAILED,
        }


def _required_positive_int(decoded: dict[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise _invalid_bidi(f"{field_name} is required")
    return value


def _optional_non_negative_int(value: object, field_name: str) -> int:
    if value is None:
        return 0
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_bidi(f"{field_name} must be a non-negative integer")
    return value


def _required_string(decoded: dict[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_bidi(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_bidi(f"{field_name} must be a string or null")
    return value


def _required_bool(decoded: dict[str, object], field_name: str) -> bool:
    value = decoded.get(field_name)
    if not isinstance(value, bool):
        raise _invalid_bidi(f"{field_name} must be a boolean")
    return value


def _optional_bool(value: object, field_name: str) -> Optional[bool]:
    if value is None:
        return None
    if not isinstance(value, bool):
        raise _invalid_bidi(f"{field_name} must be a boolean or null")
    return value


def _bidi_state(value: str) -> BidiState:
    try:
        return BidiState(value)
    except ValueError as exc:
        raise _invalid_bidi(f"unknown bidi state: {value}", exc) from exc


def _invalid_bidi(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="bidi",
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
