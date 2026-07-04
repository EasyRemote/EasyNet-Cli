"""Public daemon Invocation transport facade.

This module is the migration surface for product code that wants JSON-friendly
daemon Invocation calls without owning C ABI loading. It wraps Runtime Core
objects; it does not implement Invocation, stream, bidi, URA, or receipt
semantics itself.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Iterable, Mapping

from .bidi import BidiFrame, BidiOutcome, BidiSession, BidiState, BidiStreamDescriptor
from .connection import (
    ConnectOptions,
    ControlDiscoveryRuntimeConnector,
    RuntimeConnection,
)
from .errors import ErrorCode, RetryHint, SDKError
from .invocation import InvocationDraft
from .runtime import InvocationCancel, InvocationFailure, InvocationResult, RuntimeClient
from .stream import StreamCancel, StreamEvent, StreamHandle


@dataclass
class DaemonInvocationTransport:
    """JSON-friendly facade over the SDK Runtime Core client."""

    runtime: RuntimeClient
    connection: RuntimeConnection | None = None
    _closed: bool = False

    @classmethod
    def from_runtime_client(
        cls, runtime: RuntimeClient
    ) -> "DaemonInvocationTransport":
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

    def invoke(self, invocation: Mapping[str, object] | InvocationDraft) -> dict[str, object]:
        """Submit one complete Invocation and return its Runtime result JSON."""

        result = self._require_open().invoke(_coerce_draft(invocation))
        return _invocation_result_dict(result)

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


def _coerce_draft(invocation: Mapping[str, object] | InvocationDraft) -> InvocationDraft:
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


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _optional_string(value: object, field_name: str) -> str:
    if value is None:
        return ""
    if not isinstance(value, str):
        raise _invalid_transport(f"{field_name} must be a string or null")
    return value


def _invalid_transport(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="transport",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )


def _closed_transport(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.CANCELLED,
        stage="transport",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )
