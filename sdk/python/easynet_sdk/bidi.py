"""Runtime Core bidirectional session state facade."""

from __future__ import annotations

import json
import threading
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Any, Optional, Protocol, runtime_checkable

from ._carrier import CarrierState, is_local_carrier_interruption
from .errors import ErrorCode, RetryHint, SDKError
from ._receipt_projection import reject_retired_top_level_receipt_alias


MAX_BIDI_BUFFERED_FRAMES = 1024
BIDI_RUNTIME_ID_FIELD = "session_id"


class BidiState(StrEnum):
    """Runtime Core bidirectional session states."""

    CREATED = "Created"
    OPENING = "Opening"
    OPEN = "Open"
    CANCEL_REQUESTED = "CancelRequested"
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

    def send(self, frame_json: bytes) -> bytes: ...

    def recv(self, timeout: float | None = None) -> bytes: ...

    def close_send(self) -> bytes: ...

    def close(self) -> None: ...

    def cancel(self, reason: str) -> bytes: ...


@dataclass(frozen=True)
class BidiFrame:
    """SDK bidi frame projection."""

    sequence: int
    kind: str
    stream_id: int = 0
    terminal: bool = False
    transport_terminal: bool = False
    payload_content_type: str = ""
    payload_base64: str = ""
    payload_json: Any = None
    error: Any = None
    admission_receipt: Any = None
    terminal_receipt: Any = None

    @classmethod
    def from_json(cls, raw: bytes | str) -> "BidiFrame":
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_bidi(f"decode bidi frame JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_bidi("bidi frame JSON must be an object")
        _reject_unknown_bidi_fields(
            decoded,
            "bidi frame",
            "sequence",
            "kind",
            "stream_id",
            "terminal",
            "transport_terminal",
            "payload_content_type",
            "payload_base64",
            "payload_json",
            "error",
            "admission_receipt",
            "terminal_receipt",
        )
        reject_retired_top_level_receipt_alias(decoded, "bidi frame", stage="bidi")
        kind = _optional_string(decoded.get("kind"), "kind")
        if not kind:
            raise _invalid_bidi("bidi frame kind is required")
        return cls(
            sequence=_required_positive_int(decoded, "sequence"),
            kind=kind,
            stream_id=_optional_non_negative_int(decoded.get("stream_id"), "stream_id"),
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
            error=decoded.get("error"),
            admission_receipt=decoded.get("admission_receipt"),
            terminal_receipt=decoded.get("terminal_receipt"),
        )

    def to_json(self) -> bytes:
        value: dict[str, object] = {
            "sequence": self.sequence,
            "kind": self.kind,
            "stream_id": self.stream_id,
            "terminal": self.terminal,
        }
        if self.transport_terminal:
            value["transport_terminal"] = self.transport_terminal
        if self.payload_content_type:
            value["payload_content_type"] = self.payload_content_type
        if self.payload_base64:
            value["payload_base64"] = self.payload_base64
        if self.payload_json is not None:
            value["payload_json"] = self.payload_json
        if self.error is not None:
            value["error"] = self.error
        if self.admission_receipt is not None:
            value["admission_receipt"] = self.admission_receipt
        if self.terminal_receipt is not None:
            value["terminal_receipt"] = self.terminal_receipt
        return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


@dataclass(frozen=True)
class BidiTerminalFrame:
    """Schema-shaped Runtime Core bidi terminal frame projection."""

    session_id: str
    frame_type: str
    seq: int
    payload: Any = None
    error: Any = None
    terminal_receipt: Any = None

    @classmethod
    def from_frame(cls, session_id: str, frame: BidiFrame) -> "BidiTerminalFrame":
        if not session_id:
            raise _invalid_bidi("session_id is required")
        if not frame.terminal:
            raise _invalid_bidi("bidi frame is not terminal")
        frame_type = _bidi_terminal_frame_type(frame)
        return cls(
            session_id=session_id,
            frame_type=frame_type,
            seq=frame.sequence,
            payload=frame.payload_json,
            error=frame.error,
            terminal_receipt=frame.terminal_receipt,
        )

    def to_json(self) -> bytes:
        value: dict[str, object] = {
            "session_id": self.session_id,
            "frame_type": self.frame_type,
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
        _reject_unknown_bidi_fields(
            decoded,
            "bidi outcome",
            "session_id",
            "state",
            "terminal",
            "reason",
        )
        state = _bidi_state(_required_string(decoded, "state"))
        if state not in {
            BidiState.CANCEL_REQUESTED,
            BidiState.HALF_CLOSED_LOCAL,
            BidiState.HALF_CLOSED_REMOTE,
            BidiState.TERMINAL,
            BidiState.CANCELLED,
            BidiState.CLOSED,
            BidiState.FAILED,
        }:
            raise _invalid_bidi("invalid bidi outcome state")
        terminal = _required_bool(decoded, "terminal")
        if state == BidiState.CANCEL_REQUESTED and terminal:
            raise _invalid_bidi("bidi cancel request must not be terminal")
        if (
            state
            in {
                BidiState.TERMINAL,
                BidiState.CANCELLED,
                BidiState.CLOSED,
                BidiState.FAILED,
            }
            and not terminal
        ):
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
    _terminal_frame: Optional[BidiTerminalFrame] = None
    _runtime_state: BidiState = field(init=False, repr=False)
    _carrier_state: CarrierState = field(
        default=CarrierState.OPEN, init=False, repr=False
    )
    _local_half_close: bool = False
    _remote_half_close: bool = False
    _lock: Any = field(
        default_factory=threading.RLock, init=False, repr=False, compare=False
    )
    _sending: bool = field(default=False, init=False, repr=False)
    _receiving: bool = field(default=False, init=False, repr=False)

    def __post_init__(self) -> None:
        self._runtime_state = self.state

    @property
    def runtime_state(self) -> BidiState:
        """Provider-observed lifecycle state, excluding local carrier close."""

        with self._lock:
            return self._runtime_state

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
        _reject_unknown_bidi_fields(
            decoded,
            "bidi open",
            "session_id",
            "state",
            "max_buffered_frames",
        )
        state = _bidi_state(
            _optional_string(decoded.get("state"), "state") or "Opening"
        )
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
        frame_json = frame.to_json()
        with self._lock:
            self._require_carrier_open_locked()
            if self._runtime_state == BidiState.HALF_CLOSED_LOCAL:
                raise SDKError(
                    code=ErrorCode.CANCELLED,
                    stage="bidi",
                    retry=RetryHint.NEVER,
                    retryable=False,
                    message="bidi send path is closed",
                )
            if self._runtime_state not in {
                BidiState.OPEN,
                BidiState.HALF_CLOSED_REMOTE,
            }:
                raise _invalid_bidi("bidi send path is closed")
            if (
                self.max_buffered_frames > 0
                and len(self.sent_frames) >= self.max_buffered_frames
            ):
                self._set_runtime_state_locked(BidiState.FAILED)
                raise _invalid_bidi("bidi send buffer limit exceeded")
            if self._sending:
                raise _invalid_bidi("bidi send is already in progress")
            self._sending = True
            transport = self.transport
        try:
            raw = transport.send(frame_json)
        except SDKError as exc:
            with self._lock:
                self._sending = False
                if self._carrier_state.is_open and not is_local_carrier_interruption(
                    exc
                ):
                    self._set_runtime_state_locked(BidiState.FAILED)
            raise
        except Exception as exc:
            with self._lock:
                self._sending = False
                if self._carrier_state.is_open and not is_local_carrier_interruption(
                    exc
                ):
                    self._set_runtime_state_locked(BidiState.FAILED)
            raise _transport_error("bidi send transport failed", exc) from exc
        try:
            ack = BidiFrame.from_json(raw)
        except SDKError:
            with self._lock:
                self._sending = False
                self._set_runtime_state_locked(BidiState.FAILED)
            raise
        with self._lock:
            self._sending = False
            if (
                self._is_runtime_terminal_locked()
                or self._runtime_state == BidiState.CANCEL_REQUESTED
            ):
                raise _invalid_bidi("bidi send completed after the send path closed")
            self._record_sent_locked(ack)
        return ack

    def receive(self, timeout: float | None = None) -> BidiFrame:
        with self._lock:
            self._require_carrier_open_locked()
            if self._is_runtime_terminal_locked():
                raise _invalid_bidi("bidi session is terminal")
            if self._receiving:
                raise _invalid_bidi("bidi recv is already in progress")
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
                    self._set_runtime_state_locked(BidiState.FAILED)
            raise
        except TimeoutError:
            with self._lock:
                self._receiving = False
            raise
        except Exception as exc:
            with self._lock:
                self._receiving = False
                if self._carrier_state.is_open:
                    self._set_runtime_state_locked(BidiState.FAILED)
            raise _transport_error("bidi recv transport failed", exc) from exc
        try:
            frame = BidiFrame.from_json(raw)
        except SDKError:
            with self._lock:
                self._receiving = False
                self._set_runtime_state_locked(BidiState.FAILED)
            raise
        with self._lock:
            self._receiving = False
            if self._is_runtime_terminal_locked():
                raise _invalid_bidi(
                    "bidi session became terminal while receive was in progress"
                )
            self._record_received_locked(frame)
            self._apply_received_state_locked(frame)
        return frame

    def close_send(self) -> BidiOutcome:
        with self._lock:
            self._require_carrier_open_locked()
            if self._runtime_state not in {
                BidiState.OPEN,
                BidiState.HALF_CLOSED_REMOTE,
            }:
                raise _invalid_bidi("bidi send path is closed")
            if self._sending:
                raise _invalid_bidi("bidi send is already in progress")
            self._sending = True
            transport = self.transport
        try:
            raw = transport.close_send()
        except SDKError as exc:
            with self._lock:
                self._sending = False
                if self._carrier_state.is_open and not is_local_carrier_interruption(
                    exc
                ):
                    self._set_runtime_state_locked(BidiState.FAILED)
            raise
        except Exception as exc:
            with self._lock:
                self._sending = False
                if self._carrier_state.is_open and not is_local_carrier_interruption(
                    exc
                ):
                    self._set_runtime_state_locked(BidiState.FAILED)
            raise _transport_error("bidi close-send transport failed", exc) from exc
        try:
            outcome = BidiOutcome.from_json(raw)
        except SDKError:
            with self._lock:
                self._sending = False
                self._set_runtime_state_locked(BidiState.FAILED)
            raise
        with self._lock:
            self._sending = False
            if outcome.state != BidiState.HALF_CLOSED_LOCAL or outcome.terminal:
                self._set_runtime_state_locked(BidiState.FAILED)
                raise _invalid_bidi(
                    "bidi close-send transport must return "
                    "HalfClosedLocal with terminal=false"
                )
            if (
                self._is_runtime_terminal_locked()
                or self._runtime_state == BidiState.CANCEL_REQUESTED
            ):
                return outcome
            self._local_half_close = True
            self._set_runtime_state_locked(BidiState.HALF_CLOSED_LOCAL)
        return outcome

    def cancel(self, reason: str = "") -> BidiOutcome:
        with self._lock:
            self._require_carrier_open_locked()
            if self._is_runtime_terminal_locked():
                raise _invalid_bidi("bidi session is terminal")
            transport = self.transport
        try:
            raw = transport.cancel(reason)
        except SDKError as exc:
            if (
                exc.code != ErrorCode.NOT_IMPLEMENTED
                and not is_local_carrier_interruption(exc)
            ):
                with self._lock:
                    self._set_runtime_state_locked(BidiState.FAILED)
            raise
        except Exception as exc:
            if not is_local_carrier_interruption(exc):
                with self._lock:
                    self._set_runtime_state_locked(BidiState.FAILED)
            raise _transport_error("bidi cancel transport failed", exc) from exc
        try:
            outcome = BidiOutcome.from_json(raw)
        except SDKError:
            with self._lock:
                self._set_runtime_state_locked(BidiState.FAILED)
            raise
        if outcome.state != BidiState.CANCEL_REQUESTED or outcome.terminal:
            with self._lock:
                self._set_runtime_state_locked(BidiState.FAILED)
            raise _invalid_bidi(
                "bidi cancel transport must return CancelRequested with terminal=false"
            )
        with self._lock:
            if self._runtime_state == BidiState.TERMINAL:
                return outcome
            if self._runtime_state == BidiState.FAILED:
                raise _invalid_bidi(
                    "bidi session failed while cancellation was in flight"
                )
            self._set_runtime_state_locked(outcome.state)
        return outcome

    def close(self) -> None:
        with self._lock:
            if self._carrier_state is CarrierState.CLOSED:
                return
            if self._carrier_state is CarrierState.CLOSING:
                raise _invalid_bidi("bidi carrier close is already in progress")
            self._carrier_state = CarrierState.CLOSING
            transport = self.transport
        try:
            transport.close()
        except SDKError:
            with self._lock:
                self._carrier_state = CarrierState.FAILED
                self.state = BidiState.FAILED
            raise
        except Exception as exc:
            with self._lock:
                self._carrier_state = CarrierState.FAILED
                self.state = BidiState.FAILED
            raise _transport_error("bidi close transport failed", exc) from exc
        with self._lock:
            self._carrier_state = CarrierState.CLOSED
            self.state = BidiState.CLOSED

    def terminal_frame(self) -> BidiTerminalFrame:
        with self._lock:
            if self._terminal_frame is None:
                raise _invalid_bidi("bidi terminal frame has not been seen")
            return self._terminal_frame

    def _record_sent_locked(self, frame: BidiFrame) -> None:
        if frame.sequence <= self._last_send_sequence:
            self._set_runtime_state_locked(BidiState.FAILED)
            raise _invalid_bidi("bidi sent frames must be strictly ordered")
        self._last_send_sequence = frame.sequence
        self.sent_frames.append(frame)

    def _record_received_locked(self, frame: BidiFrame) -> None:
        if frame.sequence <= self._last_recv_sequence:
            self._set_runtime_state_locked(BidiState.FAILED)
            raise _invalid_bidi("bidi received frames must be strictly ordered")
        if (
            self.max_buffered_frames > 0
            and len(self.received_frames) >= self.max_buffered_frames
        ):
            self._set_runtime_state_locked(BidiState.FAILED)
            raise _invalid_bidi("bidi receive buffer limit exceeded")
        self._last_recv_sequence = frame.sequence
        self.received_frames.append(frame)

    def _apply_received_state_locked(self, frame: BidiFrame) -> None:
        if frame.transport_terminal:
            if frame.terminal:
                self._terminal_frame = BidiTerminalFrame.from_frame(
                    self.session_id, frame
                )
            self._carrier_state = CarrierState.FAILED
            self.state = BidiState.FAILED
        elif frame.terminal:
            self._terminal_frame = BidiTerminalFrame.from_frame(self.session_id, frame)
            self._set_runtime_state_locked(BidiState.TERMINAL)
        elif frame.kind == "remote_close_send":
            self._remote_half_close = True
            if not self._local_half_close:
                self._set_runtime_state_locked(BidiState.HALF_CLOSED_REMOTE)
        elif self._runtime_state == BidiState.OPENING:
            self._set_runtime_state_locked(BidiState.OPEN)

    def _set_runtime_state_locked(self, state: BidiState) -> None:
        self._runtime_state = state
        if self._carrier_state.is_open:
            self.state = state

    def _require_carrier_open_locked(self) -> None:
        if not self._carrier_state.is_open:
            raise _invalid_bidi("bidi carrier is closed")

    def _is_runtime_terminal_locked(self) -> bool:
        return self._runtime_state in {
            BidiState.TERMINAL,
            BidiState.CANCELLED,
            BidiState.FAILED,
        }


def _required_positive_int(decoded: dict[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise _invalid_bidi(f"{field_name} is required")
    return value


def _reject_unknown_bidi_fields(
    decoded: dict[str, object], projection: str, *allowed_fields: str
) -> None:
    allowed = set(allowed_fields)
    for field_name in decoded:
        if field_name not in allowed:
            raise _invalid_bidi(
                f"{projection} contains noncanonical field {field_name}"
            )


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


def _bidi_terminal_frame_type(frame: BidiFrame) -> str:
    if frame.kind in {"terminal", "error", "cancelled"}:
        return frame.kind
    if frame.error is not None:
        return "error"
    return "terminal"


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
        code=ErrorCode.ROUTE_UNAVAILABLE,
        stage="transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=message,
        cause=cause,
    )
