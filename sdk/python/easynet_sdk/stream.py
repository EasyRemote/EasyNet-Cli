"""Runtime Core stream state facade."""

from __future__ import annotations

import base64
import json
import threading
import warnings
import weakref
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Any, BinaryIO, Callable, Optional, Protocol, runtime_checkable

from ._carrier import CarrierState, is_local_carrier_interruption
from .errors import ErrorCode, RetryHint, SDKError
from ._receipt_projection import reject_retired_top_level_receipt_alias


MAX_STREAM_BUFFERED_EVENTS = 1024

_CANONICAL_RUNTIME_STATES = frozenset(
    (
        "Accepted",
        "Admitted",
        "Dispatched",
        "Running",
        "Completed",
        "Failed",
        "TimedOut",
        "Cancelled",
    )
)


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


@dataclass(frozen=True)
class RawStreamPacket:
    """Typed ABI v8 frame with raw payload and sparse JSON sidecars."""

    sequence: int
    kind: str
    state: str
    terminal: bool
    transport_terminal: bool
    elapsed_ms: int
    payload_content_type: str
    payload: bytes
    admission_receipt_json: bytes = b""
    terminal_receipt_json: bytes = b""
    error_json: bytes = b""


class _LeaseState:
    """One explicit native lease reference shared with its finalizer."""

    def __init__(self, lease_id: int, release: Callable[[], None]) -> None:
        self.lease_id = lease_id
        self._release = release
        self._released = False
        self._lock = threading.Lock()

    @property
    def released(self) -> bool:
        with self._lock:
            return self._released

    def release(self) -> bool:
        with self._lock:
            if self._released:
                return False
            self._released = True
            self._release()
        return True

    def use(self, operation: Callable[[], Any]) -> tuple[bool, Any]:
        with self._lock:
            if self._released:
                return False, None
            return True, operation()

    def consume(self, operation: Callable[[], Any]) -> tuple[bool, Any]:
        with self._lock:
            if self._released:
                return False, None
            self._released = True
            try:
                return True, operation()
            finally:
                self._release()


def _finalize_leased_payload(state: _LeaseState) -> None:
    if state.released:
        return
    warnings.warn(
        "LeasedPayload was not explicitly released; releasing it from the finalizer",
        ResourceWarning,
        stacklevel=2,
    )
    try:
        state.release()
    except Exception:
        # Finalization is a diagnostic safety net. Normal correctness relies on
        # release(), context management, or a consuming copy/write operation.
        return


class LeasedPayload:
    """Explicitly owned ABI v9 payload lease.

    The native pointer is deliberately not public. Callers either copy into
    owned Python storage, write into caller-owned storage, or release the lease.
    Every consuming operation releases this reference even when copying/writing
    raises.
    """

    def __init__(
        self,
        *,
        lease_id: int,
        length: int,
        copy_bytes: Callable[[], bytes],
        copy_into: Callable[[memoryview], int],
        retain: Callable[[], "LeasedPayload"],
        release: Callable[[], None],
    ) -> None:
        if lease_id <= 0:
            raise _invalid_stream("leased payload lease_id must be positive")
        if length <= 0:
            raise _invalid_stream("leased payload length must be positive")
        self._length = length
        self._copy_bytes = copy_bytes
        self._copy_into = copy_into
        self._retain = retain
        self._state = _LeaseState(lease_id, release)
        self._finalizer = weakref.finalize(self, _finalize_leased_payload, self._state)

    @property
    def lease_id(self) -> int:
        """Opaque lease identity for diagnostics; it is never a memory address."""

        return self._state.lease_id

    @property
    def length(self) -> int:
        return self._length

    @property
    def released(self) -> bool:
        return self._state.released

    def retain(self) -> "LeasedPayload":
        """Return a separately releasable reference to the same immutable bytes."""

        live, retained = self._state.use(self._retain)
        if not live:
            raise _invalid_stream("leased payload is released")
        return retained

    def release(self) -> None:
        """Release this reference exactly once; repeated calls are harmless."""

        self._state.release()
        self._finalizer.detach()

    def to_bytes(self) -> bytes:
        """Copy into owned ``bytes`` and consume this lease reference."""

        def copy() -> bytes:
            value = self._copy_bytes()
            if len(value) != self._length:
                raise _invalid_stream("leased payload copy returned an invalid length")
            return value

        live, value = self._state.consume(copy)
        self._finalizer.detach()
        if not live:
            raise _invalid_stream("leased payload is released")
        return value

    def write_into(self, destination: object) -> int:
        """Copy into writable caller-owned storage and consume this lease."""

        def copy() -> int:
            view = memoryview(destination)
            if view.readonly:
                raise _invalid_stream("leased payload destination must be writable")
            try:
                byte_view = view.cast("B")
            except (TypeError, ValueError) as exc:
                raise _invalid_stream(
                    "leased payload destination must be contiguous byte-addressable storage",
                    exc,
                ) from exc
            if len(byte_view) < self._length:
                raise _invalid_stream("leased payload destination is too small")
            written = self._copy_into(byte_view[: self._length])
            if written != self._length:
                raise _invalid_stream("leased payload write returned an invalid length")
            return written

        live, written = self._state.consume(copy)
        self._finalizer.detach()
        if not live:
            raise _invalid_stream("leased payload is released")
        return written

    def write_to(self, destination: BinaryIO) -> int:
        """Write owned chunks to a binary sink and consume this lease."""

        def write() -> int:
            if not callable(getattr(destination, "write", None)):
                raise _invalid_stream(
                    "leased payload destination must provide write(bytes)"
                )
            payload = self._copy_bytes()
            if len(payload) != self._length:
                raise _invalid_stream("leased payload copy returned an invalid length")
            written = destination.write(payload)
            if written is None:
                return self._length
            if not isinstance(written, int) or isinstance(written, bool):
                raise _invalid_stream("leased payload writer returned an invalid count")
            if written != self._length:
                raise _invalid_stream("leased payload writer did not consume the full payload")
            return written

        live, written = self._state.consume(write)
        self._finalizer.detach()
        if not live:
            raise _invalid_stream("leased payload is released")
        return written

    def __enter__(self) -> "LeasedPayload":
        self._require_live()
        return self

    def __exit__(self, exc_type: object, exc: object, traceback: object) -> None:
        self.release()

    def _require_live(self) -> None:
        if self.released:
            raise _invalid_stream("leased payload is released")


@dataclass(frozen=True)
class LeasedStreamEvent:
    """ABI v9 stream event whose non-empty payload has explicit ownership."""

    sequence: int
    kind: str
    state: str
    terminal: bool
    transport_terminal: bool
    elapsed_ms: int
    payload_content_type: str
    payload: LeasedPayload | None = field(default=None, compare=False, repr=False)
    error: Any = None
    admission_receipt: Any = None
    terminal_receipt: Any = None

    def release(self) -> None:
        if self.payload is not None:
            self.payload.release()

    def to_owned(self) -> "StreamEvent":
        """Copy the payload and return the existing owned v8-style projection."""

        payload_bytes = self.payload.to_bytes() if self.payload is not None else b""
        payload_json: Any = None
        if payload_bytes and "json" in self.payload_content_type.lower():
            try:
                payload_json = json.loads(payload_bytes.decode("utf-8"))
            except Exception as exc:
                raise _invalid_stream(
                    f"decode leased JSON stream payload: {exc}", exc
                ) from exc
        return StreamEvent(
            sequence=self.sequence,
            kind=self.kind,
            state=self.state,
            terminal=self.terminal,
            transport_terminal=self.transport_terminal,
            payload_content_type=self.payload_content_type,
            payload_bytes=payload_bytes,
            payload_json=payload_json,
            elapsed_ms=self.elapsed_ms,
            error=self.error,
            admission_receipt=self.admission_receipt,
            terminal_receipt=self.terminal_receipt,
        )

    def __enter__(self) -> "LeasedStreamEvent":
        return self

    def __exit__(self, exc_type: object, exc: object, traceback: object) -> None:
        self.release()


@runtime_checkable
class LeasedStreamTransport(Protocol):
    """Transport for the explicit ABI v9 leased-event surface."""

    def recv(self, timeout: float | None = None) -> LeasedStreamEvent: ...

    def cancel(self, reason: str) -> bytes: ...

    def close(self) -> None: ...


@runtime_checkable
class StreamTransport(Protocol):
    """Concrete stream frame transport supplied by the integration layer."""

    def recv(self, timeout: float | None = None) -> bytes | RawStreamPacket: ...

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
    payload_bytes: bytes = b""
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
        reject_retired_top_level_receipt_alias(decoded, "stream event", stage="stream")
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
        payload_base64 = (
            _optional_string(decoded.get("payload_base64"), "payload_base64") or ""
        )
        payload_bytes = _payload_base64_bytes(payload_base64)
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
            payload_base64=payload_base64,
            payload_bytes=payload_bytes,
            payload_json=decoded.get("payload_json"),
            elapsed_ms=_optional_non_negative_int(
                decoded.get("elapsed_ms"), "elapsed_ms"
            ),
            error=error,
            admission_receipt=decoded.get("admission_receipt"),
            terminal_receipt=decoded.get("terminal_receipt"),
        )

    @classmethod
    def from_raw_packet(cls, packet: RawStreamPacket) -> "StreamEvent":
        if not isinstance(packet.sequence, int) or isinstance(packet.sequence, bool) or packet.sequence <= 0:
            raise _invalid_stream("binary stream frame sequence must be positive")
        if not isinstance(packet.kind, str) or packet.kind not in {
            "data",
            "terminal",
            "error",
            "cancelled",
            "timeout",
            "receipt_verification_error",
        }:
            raise _invalid_stream("binary stream frame kind is not canonical")
        if not isinstance(packet.state, str) or packet.state not in _CANONICAL_RUNTIME_STATES:
            raise _invalid_stream("binary stream frame state is not canonical")
        if not isinstance(packet.terminal, bool) or not isinstance(packet.transport_terminal, bool):
            raise _invalid_stream("binary stream frame terminal flags must be booleans")
        if not isinstance(packet.elapsed_ms, int) or isinstance(packet.elapsed_ms, bool) or packet.elapsed_ms < 0:
            raise _invalid_stream("binary stream frame elapsed_ms must be non-negative")
        if not isinstance(packet.payload_content_type, str):
            raise _invalid_stream("binary stream frame content type must be a string")
        if not isinstance(packet.payload, bytes):
            raise _invalid_stream("binary stream frame payload must be bytes")
        admission_receipt = _decode_binary_sidecar(
            packet.admission_receipt_json, "admission_receipt"
        )
        terminal_receipt = _decode_binary_sidecar(
            packet.terminal_receipt_json, "terminal_receipt"
        )
        error = _decode_binary_sidecar(packet.error_json, "error")
        payload_json: Any = None
        if packet.payload and "json" in packet.payload_content_type.lower():
            try:
                payload_json = json.loads(packet.payload.decode("utf-8"))
            except Exception as exc:
                raise _invalid_stream(
                    f"decode raw JSON stream payload: {exc}", exc
                ) from exc
        return cls(
            sequence=packet.sequence,
            kind=packet.kind,
            state=packet.state,
            terminal=packet.terminal,
            transport_terminal=packet.transport_terminal,
            payload_content_type=packet.payload_content_type,
            payload_bytes=packet.payload,
            payload_json=payload_json,
            elapsed_ms=packet.elapsed_ms,
            error=error,
            admission_receipt=admission_receipt,
            terminal_receipt=terminal_receipt,
        )


def _decode_binary_sidecar(raw: bytes, field_name: str) -> dict[str, Any] | None:
    if not isinstance(raw, bytes):
        raise _invalid_stream(f"binary stream frame {field_name} sidecar must be bytes")
    if not raw:
        return None
    try:
        value = json.loads(raw.decode("utf-8"))
    except Exception as exc:
        raise _invalid_stream(f"decode binary stream {field_name} sidecar: {exc}", exc) from exc
    if not isinstance(value, dict):
        raise _invalid_stream(f"binary stream {field_name} sidecar must be an object")
    return value


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
class LeasedStreamHandle:
    """Ordered ABI v9 stream whose payload ownership stays explicit."""

    stream_id: str
    transport: LeasedStreamTransport
    state: StreamState = StreamState.OPENING
    max_buffered_events: int = MAX_STREAM_BUFFERED_EVENTS
    _last_sequence: int = field(default=0, init=False, repr=False)
    _terminal_seen: bool = field(default=False, init=False, repr=False)
    _receiving: bool = field(default=False, init=False, repr=False)
    _lock: Any = field(
        default_factory=threading.RLock, init=False, repr=False, compare=False
    )

    @classmethod
    def from_json(
        cls, transport: LeasedStreamTransport, raw: bytes | str
    ) -> "LeasedStreamHandle":
        if transport is None:
            raise _invalid_stream("leased stream transport is required")
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text) if text else {}
        except Exception as exc:
            raise _invalid_stream(f"decode leased stream open JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_stream("leased stream open JSON must be an object")
        _reject_unknown_stream_fields(
            decoded,
            "leased stream open",
            "stream_id",
            "state",
            "max_buffered_events",
        )
        state = _stream_state(
            _optional_string(decoded.get("state"), "state") or "Opening"
        )
        if state not in {StreamState.OPENING, StreamState.OPEN}:
            raise _invalid_stream("leased stream open state must be Opening or Open")
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

    def next(self, timeout: float | None = None) -> LeasedStreamEvent:
        with self._lock:
            if self.state == StreamState.CLOSED:
                raise _invalid_stream("leased stream is closed")
            if self._terminal_seen:
                raise _invalid_stream("leased stream is terminal")
            if self._receiving:
                raise _invalid_stream("leased stream recv is already in progress")
            self._receiving = True
        event: LeasedStreamEvent | None = None
        try:
            event = self.transport.recv(timeout)
            with self._lock:
                if event.sequence <= self._last_sequence:
                    self.state = StreamState.FAILED
                    raise _invalid_stream(
                        "leased stream event sequence must be strictly increasing"
                    )
                self._last_sequence = event.sequence
                if event.terminal:
                    self._terminal_seen = True
                    self.state = StreamState.TERMINAL_FRAME_SEEN
                elif event.transport_terminal:
                    self.state = StreamState.FAILED
                else:
                    self.state = StreamState.OPEN
                return event
        except Exception:
            if event is not None:
                event.release()
            raise
        finally:
            with self._lock:
                self._receiving = False

    def cancel(self, reason: str = "") -> StreamCancel:
        with self._lock:
            if self.state == StreamState.CLOSED:
                raise _invalid_stream("leased stream is closed")
        outcome = StreamCancel.from_json(self.transport.cancel(reason))
        with self._lock:
            self.state = outcome.state
        return outcome

    def close(self) -> None:
        with self._lock:
            if self.state == StreamState.CLOSED:
                return
            self.state = StreamState.CLOSED
        self.transport.close()

    def __enter__(self) -> "LeasedStreamHandle":
        return self

    def __exit__(self, exc_type: object, exc: object, traceback: object) -> None:
        self.close()


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
            event = (
                StreamEvent.from_raw_packet(raw)
                if isinstance(raw, RawStreamPacket)
                else StreamEvent.from_json(raw)
            )
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


def _payload_base64_bytes(payload_base64: str) -> bytes:
    if not payload_base64:
        return b""
    try:
        return base64.b64decode(payload_base64, validate=True)
    except Exception as exc:
        raise _invalid_stream(
            f"payload_base64 must be canonical base64: {exc}", exc
        ) from exc


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
