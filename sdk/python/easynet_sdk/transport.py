"""Public daemon Invocation transport facade.

This module is the migration surface for product code that wants JSON-friendly
daemon Invocation calls without owning C ABI loading. It wraps Runtime Core
objects; it does not implement Invocation, stream, bidi, URA, or receipt
semantics itself.
"""

from __future__ import annotations

import contextlib
import base64
import json
import queue
import threading
from dataclasses import dataclass
from typing import Any, Callable, Iterable, Iterator, Mapping, Protocol, cast

from .bidi import BidiFrame, BidiOutcome, BidiSession, BidiState, BidiStreamDescriptor
from .connection import (
    ConnectOptions,
    ControlDiscoveryRuntimeConnector,
    RuntimeConnection,
)
from .errors import ErrorCode, RetryHint, SDKError
from .invocation import InvocationDraft
from .runtime import InvocationFailure, InvocationResult, PrepareOptions, RuntimeClient
from .signing import Signer
from .stream import StreamCancel, StreamEvent, StreamHandle


@dataclass
class DaemonInvocationTransport:
    """JSON-friendly facade over the SDK Runtime Core client."""

    runtime: RuntimeClient
    connection: RuntimeConnection | None = None
    _closed: bool = False

    @classmethod
    def from_runtime_client(cls, runtime: RuntimeClient) -> "DaemonInvocationTransport":
        """Wrap an existing Runtime Core client."""

        return cls(runtime)

    @classmethod
    def connect(
        cls,
        *,
        control_path: str = "",
        library_path: str | None = None,
        options: ConnectOptions = ConnectOptions(),
    ) -> "DaemonInvocationTransport":
        """Open a stateful SDK-owned Runtime Core session."""

        from . import _cabi

        control_path = options.control_path or control_path
        connection = RuntimeConnection(
            ControlDiscoveryRuntimeConnector(
                _cabi.open_cabi_runtime_connector(library_path=library_path),
                control_path=control_path,
            )
        )
        connection.connect(
            ConnectOptions(
                endpoint=options.endpoint,
                control_path=control_path,
                dial_timeout_ms=options.dial_timeout_ms,
                invoke_timeout_ms=options.invoke_timeout_ms,
                max_message_bytes=options.max_message_bytes,
                reconnect=options.reconnect,
            )
        )
        return cls(
            runtime=connection.runtime_client(),
            connection=connection,
        )

    @classmethod
    def connect_direct(
        cls,
        *,
        control_path: str = "",
        options: ConnectOptions = ConnectOptions(),
    ) -> "DaemonInvocationTransport":
        """Open a direct daemon Axon gRPC-over-UDS Runtime Core session."""

        from .direct_runtime import DirectDaemonRuntimeConnector

        control_path = options.control_path or control_path
        connection = RuntimeConnection(
            DirectDaemonRuntimeConnector(control_path=control_path)
        )
        connection.connect(
            ConnectOptions(
                endpoint=options.endpoint,
                control_path=control_path,
                dial_timeout_ms=options.dial_timeout_ms,
                invoke_timeout_ms=options.invoke_timeout_ms,
                max_message_bytes=options.max_message_bytes,
                reconnect=options.reconnect,
            )
        )
        return cls(
            runtime=connection.runtime_client(),
            connection=connection,
        )

    def invoke(
        self, invocation: Mapping[str, object] | InvocationDraft
    ) -> dict[str, object]:
        """Submit one complete Invocation and return its Runtime result JSON."""

        result = self._require_open().invoke(_coerce_draft(invocation))
        return _invocation_result_dict(result)

    def invoke_signed(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        *,
        signer: Signer | None,
        options: PrepareOptions = PrepareOptions(require_user_sig=True),
    ) -> dict[str, object]:
        """Prepare, sign, submit, await, and release one signed Invocation."""

        if signer is None:
            raise _missing_required_signer()
        runtime = self._require_open()
        signed, _material = runtime.prepare_and_sign(
            _coerce_draft(invocation),
            signer,
            options,
        )
        handle = runtime.submit_signed(signed)
        try:
            result = runtime.await_result(handle)
            return _invocation_result_dict(result)
        finally:
            with contextlib.suppress(Exception):
                runtime.close_handle(handle)

    def stream(
        self, invocation: Mapping[str, object] | InvocationDraft
    ) -> "DaemonFrameStream":
        """Open a server-stream Invocation."""

        handle = self._require_open().invoke_stream(_coerce_draft(invocation))
        return DaemonFrameStream(handle)

    def bidi(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        streams: Iterable[Mapping[str, object] | BidiStreamDescriptor] = (),
    ) -> "DaemonBidiChannel":
        """Open a bidirectional Invocation session."""

        session = self._require_open().open_bidi(
            _coerce_draft(invocation),
            tuple(_coerce_stream_descriptor(stream) for stream in streams),
        )
        return DaemonBidiChannel(session)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        if self.connection is not None:
            self.connection.close()
            return
        self.runtime.close()

    def __enter__(self) -> "DaemonInvocationTransport":
        self._require_open()
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()

    def _require_open(self) -> RuntimeClient:
        if self._closed:
            raise _closed_transport("daemon invocation transport is closed")
        return self.runtime


@dataclass
class InvocationResultAdapter:
    """Runtime result adapter over the SDK daemon Invocation transport."""

    transport: DaemonInvocationTransport

    @classmethod
    def from_runtime_client(cls, runtime: RuntimeClient) -> "InvocationResultAdapter":
        return cls(DaemonInvocationTransport.from_runtime_client(runtime))

    @classmethod
    def connect(
        cls,
        *,
        control_path: str = "",
        library_path: str | None = None,
        options: ConnectOptions = ConnectOptions(),
    ) -> "InvocationResultAdapter":
        return cls(
            DaemonInvocationTransport.connect(
                control_path=control_path,
                library_path=library_path,
                options=options,
            )
        )

    def invoke(
        self, invocation: Mapping[str, object] | InvocationDraft
    ) -> dict[str, object]:
        """Submit one complete Invocation and return runtime result adapter shape."""

        return _result_response_dict(self.transport.invoke(invocation))

    def invoke_signed(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        *,
        signer: Signer | None,
        options: PrepareOptions = PrepareOptions(require_user_sig=True),
    ) -> dict[str, object]:
        """Submit a signed Invocation and return runtime result adapter shape."""

        return _result_response_dict(
            self.transport.invoke_signed(invocation, signer=signer, options=options)
        )

    def stream(
        self, invocation: Mapping[str, object] | InvocationDraft
    ) -> "DaemonFrameStream":
        return self.transport.stream(invocation)

    def bidi(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        streams: Iterable[Mapping[str, object] | BidiStreamDescriptor] = (),
    ) -> "DaemonBidiChannel":
        return self.transport.bidi(invocation, streams)

    def close(self) -> None:
        self.transport.close()

    def __enter__(self) -> "InvocationResultAdapter":
        self.transport._require_open()
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()


class UnaryInvocationTransport(Protocol):
    """Minimal unary transport contract owned by the SDK dispatch pool."""

    def invoke(
        self, invocation: Mapping[str, object] | InvocationDraft
    ) -> Mapping[str, object]:
        """Submit one runtime-shaped unary Invocation."""

    def invoke_signed(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        *,
        signer: Signer | None,
        options: PrepareOptions = PrepareOptions(require_user_sig=True),
    ) -> Mapping[str, object]:
        """Submit one runtime-shaped signed unary Invocation."""

    def close(self) -> None:
        """Release the underlying daemon transport."""


class UnaryDispatchPool:
    """SDK-owned single-flight unary wait/retire state machine.

    Product facades may impose a client-side wait budget, but they must not own
    daemon handle reuse or delayed close rules. This pool keeps one reusable
    unary transport for owned daemon sessions, retires it after a timed-out
    caller wait, and closes retired transports only after the active daemon call
    returns.
    """

    def __init__(
        self,
        transport_factory: Callable[[], UnaryInvocationTransport],
        *,
        owned: bool = True,
    ) -> None:
        self._transport_factory = transport_factory
        self._owned = owned
        self._lock = threading.Lock()
        self._flight_lock = threading.Lock()
        self._transport: UnaryInvocationTransport | None = None
        self._retired: set[int] = set()

    @classmethod
    def from_transport(cls, transport: UnaryInvocationTransport) -> "UnaryDispatchPool":
        """Wrap an externally-owned transport without closing or retiring it."""

        return cls(lambda: transport, owned=False)

    def invoke(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        *,
        timeout: float | None = None,
    ) -> dict[str, object]:
        return self._invoke_with_transport(
            lambda transport: transport.invoke(invocation),
            timeout=timeout,
        )

    def invoke_signed(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        *,
        signer: Signer | None,
        options: PrepareOptions = PrepareOptions(require_user_sig=True),
        timeout: float | None = None,
    ) -> dict[str, object]:
        return self._invoke_with_transport(
            lambda transport: transport.invoke_signed(
                invocation,
                signer=signer,
                options=options,
            ),
            timeout=timeout,
        )

    def _invoke_with_transport(
        self,
        operation: Callable[[UnaryInvocationTransport], Mapping[str, object]],
        *,
        timeout: float | None,
    ) -> dict[str, object]:
        result: queue.Queue[tuple[bool, Mapping[str, object] | BaseException]] = (
            queue.Queue(maxsize=1)
        )
        timed_out = threading.Event()
        active_transport: list[UnaryInvocationTransport | None] = [None]

        def invoke_on_transport() -> None:
            transport: UnaryInvocationTransport | None = None
            try:
                with self._flight_lock:
                    if timed_out.is_set():
                        return
                    transport = self._connected()
                    active_transport[0] = transport
                    if timed_out.is_set():
                        self._retire(transport)
                        return
                    result.put((True, operation(transport)))
            except BaseException as exc:
                result.put((False, exc))
            finally:
                if transport is not None:
                    active_transport[0] = None
                    retired = self._take_retired(transport)
                    if self._owned and (timed_out.is_set() or retired):
                        with contextlib.suppress(BaseException):
                            transport.close()

        threading.Thread(
            target=invoke_on_transport,
            name="easynet-sdk-unary",
            daemon=True,
        ).start()
        try:
            ok, payload = result.get(timeout=timeout)
        except queue.Empty:
            timed_out.set()
            transport = active_transport[0]
            if transport is not None:
                self._retire(transport)
            raise SDKError(
                code=ErrorCode.TIMEOUT,
                stage="runtime_transport",
                retry=RetryHint.SAFE,
                retryable=True,
                message=(
                    f"no response within {timeout}s — the server-side execution "
                    "is still governed by the ability's timeout_seconds"
                ),
                details={
                    "reason": "client_wait_timeout",
                    "timeout_seconds": timeout,
                },
            ) from None
        if not ok:
            assert isinstance(payload, BaseException)
            raise payload
        return dict(cast(Mapping[str, object], payload))

    def close(self) -> None:
        if not self._owned:
            return
        if self._flight_lock.acquire(blocking=False):
            try:
                self._close_idle()
            finally:
                self._flight_lock.release()
            return
        self._retire_active()

    @property
    def current_transport(self) -> UnaryInvocationTransport | None:
        """Return the current reusable transport for tests/diagnostics."""

        with self._lock:
            return self._transport

    def connected_transport(self) -> UnaryInvocationTransport:
        """Return the current reusable transport, opening one if needed."""

        return self._connected()

    def _connected(self) -> UnaryInvocationTransport:
        if not self._owned:
            return self._transport_factory()
        with self._lock:
            if self._transport is None:
                self._transport = self._transport_factory()
            return self._transport

    def _close_idle(self) -> None:
        with self._lock:
            transport = self._transport
            self._transport = None
        if transport is not None:
            transport.close()

    def _retire_active(self) -> None:
        with self._lock:
            if self._transport is not None:
                self._retired.add(id(self._transport))
                self._transport = None

    def _retire(self, transport: UnaryInvocationTransport) -> None:
        if not self._owned:
            return
        with self._lock:
            if self._transport is transport:
                self._transport = None
            self._retired.add(id(transport))

    def _take_retired(self, transport: UnaryInvocationTransport) -> bool:
        with self._lock:
            transport_id = id(transport)
            if transport_id not in self._retired:
                return False
            self._retired.remove(transport_id)
            return True


@dataclass(frozen=True)
class StreamValue:
    """One SDK-projected stream item."""

    value: Any


class StreamValueAdapter:
    """SDK-owned stream frame projection.

    The adapter consumes generic daemon stream frames and yields
    ability values. It keeps terminal-frame, timeout, wire-error, and payload
    projection rules out of product facades.
    """

    _NO_VALUE = object()

    def __init__(self, frames: "FrameStream", *, timeout: float | None = None) -> None:
        self._frames = frames
        self._timeout = timeout

    def __iter__(self) -> Iterator[StreamValue]:
        try:
            for frame in self._raw_frames():
                error = frame.get("error")
                if error:
                    raise _remote_wire_error(error)
                value = self._frame_value(frame)
                stream_error = _stream_error_payload(value)
                if stream_error is not None:
                    raise _remote_wire_error(stream_error)
                if value is not self._NO_VALUE:
                    yield StreamValue(value)
                if frame.get("terminal") is True:
                    return
        finally:
            self.close()

    def close(self) -> None:
        self._frames.close()

    def _raw_frames(self) -> Iterator[Mapping[str, object]]:
        recv = getattr(self._frames, "recv", None)
        if not callable(recv):
            yield from self._frames
            return
        while True:
            try:
                frame = recv(timeout=self._timeout)
            except TimeoutError:
                raise SDKError(
                    code=ErrorCode.TIMEOUT,
                    stage="stream",
                    retry=RetryHint.SAFE,
                    retryable=True,
                    message=(
                        f"no stream frame within {self._timeout}s — the server-side "
                        "execution is still governed by the ability's timeout_seconds"
                    ),
                    details={
                        "reason": "client_wait_timeout",
                        "timeout_seconds": self._timeout,
                    },
                ) from None
            if frame is None:
                return
            yield frame

    def _frame_value(self, frame: Mapping[str, object]) -> Any:
        if (
            frame.get("terminal") is True
            and frame.get("payload_json") is None
            and not frame.get("payload_base64")
        ):
            return self._NO_VALUE
        if "payload_json" in frame and (
            frame.get("payload_json") is not None
            or frame.get("content_type") == "application/json"
        ):
            return frame["payload_json"]
        encoded = frame.get("payload_base64")
        if isinstance(encoded, str) and encoded:
            try:
                return base64.b64decode(encoded)
            except Exception as exc:
                raise SDKError(
                    code=ErrorCode.INVALID_ARGUMENT,
                    stage="stream",
                    retry=RetryHint.NEVER,
                    retryable=False,
                    message=f"decode stream payload_base64: {exc}",
                    cause=exc,
                ) from exc
        return self._NO_VALUE


class FrameStream(Protocol):
    """Frame stream shape consumed by `StreamValueAdapter`."""

    def recv(self, timeout: float | None = None) -> Mapping[str, object] | None: ...

    def close(self) -> None: ...

    def __iter__(self) -> Iterator[Mapping[str, object]]: ...


class BidiChannel(Protocol):
    """Bidi channel shape consumed by `BidiSessionAdapter`."""

    def send(self, frame: Mapping[str, object]) -> object: ...

    def recv(self, timeout: float | None = None) -> Mapping[str, object] | None: ...

    def close(self) -> None: ...

    def cancel(self, reason: str = "") -> object: ...


class BidiSessionAdapter:
    """SDK-owned bidi session facade.

    Public session API is intentionally small, but the lifecycle
    rules are Runtime Core concerns: an open session cannot be simply dropped,
    timeout is a typed client wait expiry, and remote wire errors must not leak
    as ordinary frames.
    """

    def __init__(
        self,
        channel: BidiChannel,
        *,
        close_reason: str = "client close",
    ) -> None:
        self._channel = channel
        self._close_reason = close_reason
        self._terminal = False
        self._closed = False

    def send(self, frame: Mapping[str, object]) -> None:
        self._require_not_closed()
        self._channel.send(frame)

    def recv(self, timeout: float | None = None) -> dict[str, object] | None:
        self._require_not_closed()
        try:
            frame = self._channel.recv(timeout=timeout)
        except StopIteration:
            self._terminal = True
            return None
        except TimeoutError:
            raise SDKError(
                code=ErrorCode.TIMEOUT,
                stage="bidi",
                retry=RetryHint.SAFE,
                retryable=True,
                message=(
                    f"no bidi frame within {timeout}s - the server-side session "
                    "is still governed by daemon/ability policy"
                ),
                details={
                    "reason": "client_wait_timeout",
                    "timeout_seconds": timeout,
                },
            ) from None
        if frame is None:
            self._terminal = True
            return None
        projected = dict(frame)
        error = projected.get("error")
        if error:
            raise _remote_wire_error(error, stage="bidi")
        if projected.get("terminal") is True:
            self._terminal = True
        return projected

    def cancel(self, reason: str = "client cancel") -> None:
        if self._closed or self._terminal:
            return
        self._channel.cancel(reason)
        self._terminal = True

    def close(self) -> None:
        if self._closed:
            return
        try:
            self._channel.close()
        except SDKError as exc:
            if self._terminal or not _is_open_bidi_close_error(exc):
                raise
            self._channel.cancel(self._close_reason)
            self._terminal = True
            self._channel.close()
        self._closed = True

    def __enter__(self) -> "BidiSessionAdapter":
        self._require_not_closed()
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()

    def _require_not_closed(self) -> None:
        if self._closed:
            raise SDKError(
                code=ErrorCode.CANCELLED,
                stage="bidi",
                retry=RetryHint.NEVER,
                retryable=False,
                message="bidi session is closed",
            )


@dataclass
class DaemonFrameStream:
    """JSON-friendly server-stream wrapper over `StreamHandle`."""

    handle: StreamHandle

    def recv(self, timeout: float | None = None) -> dict[str, object]:
        return _stream_event_dict(self.handle.next(timeout))

    def cancel(self, reason: str = "") -> dict[str, object]:
        return _stream_cancel_dict(self.handle.cancel(reason))

    def close(self) -> None:
        self.handle.close()

    def __iter__(self):
        while True:
            event = self.recv()
            yield event
            if event.get("terminal") is True:
                return

    def __enter__(self) -> "DaemonFrameStream":
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()


@dataclass
class DaemonBidiChannel:
    """JSON-friendly bidi wrapper over `BidiSession`."""

    session: BidiSession

    def send(self, frame: Mapping[str, object] | BidiFrame) -> dict[str, object]:
        return _bidi_frame_dict(self.session.send(_coerce_bidi_frame(frame)))

    def recv(self, timeout: float | None = None) -> dict[str, object]:
        return _bidi_frame_dict(self.session.receive(timeout))

    def receive(self, timeout: float | None = None) -> dict[str, object]:
        return self.recv(timeout)

    def close_send(self) -> dict[str, object]:
        return _bidi_outcome_dict(self.session.close_send())

    def cancel(self, reason: str = "") -> dict[str, object]:
        return _bidi_outcome_dict(self.session.cancel(reason))

    def close(self) -> None:
        self.session.close()

    def __enter__(self) -> "DaemonBidiChannel":
        return self

    def __exit__(self, *exc_info: object) -> None:
        if self.session.state not in {
            BidiState.TERMINAL,
            BidiState.CANCELLED,
            BidiState.FAILED,
            BidiState.CLOSED,
        }:
            self.session.cancel("context manager exit")
        self.close()


def _coerce_draft(
    invocation: Mapping[str, object] | InvocationDraft
) -> InvocationDraft:
    if isinstance(invocation, InvocationDraft):
        return invocation
    if not isinstance(invocation, Mapping):
        raise _invalid_transport("invocation must be a mapping or InvocationDraft")
    return InvocationDraft.from_json(_json_bytes(dict(invocation)))


def _coerce_stream_descriptor(
    stream: Mapping[str, object] | BidiStreamDescriptor,
) -> BidiStreamDescriptor:
    if isinstance(stream, BidiStreamDescriptor):
        return stream
    if not isinstance(stream, Mapping):
        raise _invalid_transport("bidi stream descriptor must be a mapping")
    stream_id = stream.get("stream_id")
    if not isinstance(stream_id, int) or isinstance(stream_id, bool) or stream_id <= 0:
        raise _invalid_transport("bidi stream_id is required")
    return BidiStreamDescriptor(
        stream_id=stream_id,
        content_type=_optional_string(stream.get("content_type"), "content_type"),
        codec_params=_optional_string(stream.get("codec_params"), "codec_params"),
        ordering=_optional_string(stream.get("ordering"), "ordering"),
    )


def _coerce_bidi_frame(frame: Mapping[str, object] | BidiFrame) -> BidiFrame:
    if isinstance(frame, BidiFrame):
        return frame
    if not isinstance(frame, Mapping):
        raise _invalid_transport("bidi frame must be a mapping or BidiFrame")
    return BidiFrame.from_json(_json_bytes(dict(frame)))


def _invocation_result_dict(result: InvocationResult) -> dict[str, object]:
    value: dict[str, object] = {
        "ok": result.ok,
        "tuple": result.tuple.to_json_dict(),
        "terminal_state": result.terminal_state,
        "output_content_type": result.output_content_type,
        "output_base64": result.output_base64,
        "output_json": result.output_json,
        "selected_node_id": result.selected_node_id,
        "scheduling_reason": result.scheduling_reason,
        "elapsed_ms": result.elapsed_ms,
        "receipt": dict(result.receipt) if result.receipt is not None else None,
        "receipt_summary": (
            _runtime_receipt_dict(result.receipt_summary)
            if result.receipt_summary is not None
            else None
        ),
        "error": _failure_dict(result.error) if result.error is not None else None,
    }
    return value


def _result_response_dict(result: Mapping[str, object]) -> dict[str, object]:
    if result.get("ok") is not True:
        error = result.get("error")
        message = "daemon invocation failed"
        if isinstance(error, Mapping) and isinstance(error.get("message"), str):
            message = error["message"]
        raise SDKError(
            code=ErrorCode.ABILITY_FAILED,
            stage="transport",
            retry=RetryHint.UNKNOWN,
            retryable=False,
            message=message,
            details={"runtime_result": dict(result)},
        )
    receipt = result.get("receipt")
    terminal_state = _terminal_state_name(result.get("terminal_state"))
    response: dict[str, object] = {
        "ok": result.get("ok") is True,
        "state": _terminal_state_code(terminal_state),
        "terminal_state": terminal_state,
        "result_content_type": _string_or_empty(result.get("output_content_type")),
        "result_base64": _string_or_empty(result.get("output_base64")),
        "result_json": result.get("output_json"),
        "selected_node_id": _string_or_empty(result.get("selected_node_id")),
        "scheduling_reason": _string_or_empty(result.get("scheduling_reason")),
        "elapsed_ms": _non_negative_int(result.get("elapsed_ms")),
        "admission_receipt": dict(receipt) if isinstance(receipt, Mapping) else None,
        "sdk_runtime_result": dict(result),
    }
    if result.get("error") is not None:
        response["error"] = result["error"]
    return response


def _terminal_state_name(value: object) -> str:
    if isinstance(value, str) and value:
        return value
    return "Unspecified"


_TERMINAL_STATE_CODES = {
    "unspecified": 0,
    "accepted": 1,
    "admitted": 2,
    "dispatched": 3,
    "running": 4,
    "completed": 5,
    "failed": 6,
    "timed_out": 7,
    "timedout": 7,
    "cancelled": 8,
    "canceled": 8,
}


def _terminal_state_code(value: str) -> int:
    normalized = value.replace("-", "_").lower()
    return _TERMINAL_STATE_CODES.get(normalized, 0)


def _runtime_receipt_dict(receipt) -> dict[str, object]:
    return {
        "receipt_id": receipt.receipt_id,
        "receipt_ura": receipt.receipt_ura,
        "invocation_id": receipt.invocation_id,
        "receipt_type": receipt.receipt_type,
        "state": receipt.state,
        "index": receipt.index,
        "timestamp_unix_ms": receipt.timestamp_unix_ms,
        "prev_receipt_hash_hex": receipt.prev_receipt_hash_hex,
        "self_hash_hex": receipt.self_hash_hex,
        "cleanup_complete": receipt.cleanup_complete,
        "reason": receipt.reason,
        "child_invocation_id": receipt.child_invocation_id,
        "has_causal_anchor": receipt.has_causal_anchor(),
        "raw": receipt.to_json_dict(),
    }


def _failure_dict(error: InvocationFailure) -> dict[str, object]:
    return {
        "code": error.code,
        "stage": error.stage,
        "message": error.message,
        "retryable": error.retryable,
    }


def _stream_event_dict(event: StreamEvent) -> dict[str, object]:
    return {
        "sequence": event.sequence,
        "kind": event.kind,
        "state": event.state,
        "terminal": event.terminal,
        "content_type": event.payload_content_type,
        "payload_content_type": event.payload_content_type,
        "payload_base64": event.payload_base64,
        "payload_json": event.payload_json,
        "error": event.error,
    }


def _stream_cancel_dict(cancel: StreamCancel) -> dict[str, object]:
    return {
        "stream_id": cancel.stream_id,
        "cancelled": cancel.cancelled,
        "state": cancel.state.value,
        "terminal": cancel.terminal,
    }


def _bidi_frame_dict(frame: BidiFrame) -> dict[str, object]:
    return json.loads(frame.to_json().decode("utf-8"))


def _bidi_outcome_dict(outcome: BidiOutcome) -> dict[str, object]:
    return {
        "session_id": outcome.session_id,
        "state": outcome.state.value,
        "terminal": outcome.terminal,
        "reason": outcome.reason,
    }


def _stream_error_payload(value: Any) -> Mapping[str, object] | None:
    if (
        isinstance(value, Mapping)
        and set(value) == {"error"}
        and isinstance(value["error"], Mapping)
        and "kind" in value["error"]
    ):
        return value["error"]
    return None


_REMOTE_ERROR_CODES = {
    "CANCELLED": ErrorCode.CANCELLED,
    "DEADLINE_EXCEEDED": ErrorCode.TIMEOUT,
    "UNAVAILABLE": ErrorCode.DAEMON_OFFLINE,
    "INVALID_ARGUMENT": ErrorCode.INVALID_ARGUMENT,
    "RESOURCE_EXHAUSTED": ErrorCode.ADMISSION_DENIED,
    "PERMISSION_DENIED": ErrorCode.PERMISSION_DENIED,
    "INTERNAL": ErrorCode.ADMISSION_DENIED,
}


def _remote_wire_error(error: object, *, stage: str = "stream") -> SDKError:
    if not isinstance(error, Mapping):
        return SDKError(
            code=ErrorCode.ABILITY_FAILED,
            stage=stage,
            retry=RetryHint.UNKNOWN,
            retryable=False,
            message="remote frame error",
            details={"reason": "remote_frame_error", "wire_error": error},
        )
    kind = error.get("kind")
    reason = error.get("reason")
    message = error.get("message")
    kind_text = kind if isinstance(kind, str) else ""
    reason_text = reason if isinstance(reason, str) else ""
    message_text = message if isinstance(message, str) else ""
    code = _REMOTE_ERROR_CODES.get(kind_text, ErrorCode.ABILITY_FAILED)
    return SDKError(
        code=code,
        stage=stage,
        retry=RetryHint.UNKNOWN,
        retryable=False,
        message=message_text or reason_text or kind_text or "remote frame error",
        details={
            "kind": kind_text,
            "reason": reason_text,
            "wire_error": dict(error),
        },
    )


def _is_open_bidi_close_error(error: SDKError) -> bool:
    if error.code is not ErrorCode.INVALID_ARGUMENT:
        return False
    reason = error.details.get("reason")
    if isinstance(reason, str) and reason in {
        "bidi_session_not_terminal",
        "session_not_terminal",
        "not_terminal",
    }:
        return True
    return "must be terminal before close" in error.message


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _optional_string(value: object, field_name: str) -> str:
    if value is None:
        return ""
    if not isinstance(value, str):
        raise _invalid_transport(f"{field_name} must be a string or null")
    return value


def _string_or_empty(value: object) -> str:
    return value if isinstance(value, str) else ""


def _non_negative_int(value: object) -> int:
    if isinstance(value, int) and not isinstance(value, bool) and value >= 0:
        return value
    return 0


def _invalid_transport(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="transport",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )


def _missing_required_signer() -> SDKError:
    return SDKError(
        code=ErrorCode.NOT_IMPLEMENTED,
        stage="runtime_signing",
        retry=RetryHint.NEVER,
        retryable=False,
        message=("Signed invocation requires a daemon-authorized SDK Signer"),
        details={"reason": "signing_path_pending"},
    )


def _closed_transport(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.CANCELLED,
        stage="transport",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )
