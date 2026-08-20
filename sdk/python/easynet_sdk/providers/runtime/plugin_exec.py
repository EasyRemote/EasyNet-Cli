"""Provider-scoped helper for runtime declarative exec plugins.

This module owns the JSON frame details used between the runtime host and a
process-backed plugin. Plugin authors should implement a handler over
`SidecarInvocation`; they should not hand-write sidecar protocol frames.
"""

from __future__ import annotations

from collections.abc import Callable, Iterable, Iterator, Mapping
from dataclasses import dataclass
import json
import sys
from types import MappingProxyType
from typing import Any, TextIO


class SidecarProtocolError(ValueError):
    """Raised when a runtime/plugin sidecar frame is structurally invalid."""


_CANONICAL_INVOCATION_FIELDS = frozenset(
    {
        "caller_ura",
        "callee_ura",
        "ability_ura",
        "subject_ura",
        "invocation_nonce",
        "causal_context",
        "args",
    }
)
_CANONICAL_OPEN_REQUEST_FIELDS = frozenset({"type", "call_id", "invocation"})
_CANONICAL_BIDI_INPUT_FIELDS = frozenset({"type", "call_id", "frame"})
_CANONICAL_CLOSE_FIELDS = frozenset({"type", "call_id", "reason"})
_CANONICAL_INVOCATION_NONCE_BYTES = 16


@dataclass(frozen=True)
class SidecarInvocation:
    """Typed view of one runtime-admitted plugin invocation."""

    call_id: str
    caller_ura: str
    callee_ura: str
    ability_ura: str
    subject_ura: str
    invocation_nonce: tuple[int, ...]
    causal_context: Mapping[str, Any]
    args: Mapping[str, Any]
    frame_type: str = "invoke"

    @classmethod
    def from_frame(
        cls, frame: Mapping[str, Any], *, expected_type: str = "invoke"
    ) -> "SidecarInvocation":
        """Project a runtime sidecar frame into a handler-facing invocation."""

        _reject_unknown_open_request_fields(frame)
        frame_type = _required_string(frame, "type")
        if frame_type != expected_type:
            raise SidecarProtocolError(
                f"exec plugin expected {expected_type} frame, got {frame_type!r}"
            )
        call_id = _required_string(frame, "call_id")
        invocation = _required_mapping(frame, "invocation")
        _reject_unknown_invocation_fields(invocation)
        _require_invocation_fields(invocation)
        nonce = _required_nonce(invocation, "invocation_nonce")
        causal_context = _immutable_mapping(
            _required_mapping(invocation, "causal_context")
        )
        args = _immutable_mapping(_required_mapping(invocation, "args"))
        return cls(
            call_id=call_id,
            caller_ura=_required_string(invocation, "caller_ura"),
            callee_ura=_required_string(invocation, "callee_ura"),
            ability_ura=_required_string(invocation, "ability_ura"),
            subject_ura=_required_string(invocation, "subject_ura"),
            invocation_nonce=nonce,
            causal_context=causal_context,
            args=args,
            frame_type=frame_type,
        )


SidecarHandler = Callable[[SidecarInvocation], Any]
SidecarStreamHandler = Callable[[SidecarInvocation], Iterable[Any]]
SidecarBidiInputFrames = Iterator[Mapping[str, Any]]
SidecarBidiHandler = Callable[[SidecarInvocation, SidecarBidiInputFrames], Iterable[Any]]


def serve_exec_plugin(
    handler: SidecarHandler,
    *,
    input_stream: TextIO | None = None,
    output_stream: TextIO | None = None,
) -> None:
    """Run one declarative exec plugin invocation.

    The runtime host writes one request frame to stdin and expects exactly one
    response frame on stdout. Handler exceptions are converted into runtime
    protocol `error` frames so tracebacks never corrupt stdout framing.
    """

    serve_plugin(
        invoke_handler=handler,
        input_stream=input_stream,
        output_stream=output_stream,
    )


def serve_stream_plugin(
    handler: SidecarStreamHandler,
    *,
    input_stream: TextIO | None = None,
    output_stream: TextIO | None = None,
    terminal_reason: str = "stream_complete",
) -> None:
    """Run one declarative stream plugin invocation.

    The runtime host writes one ``stream_open`` request. The handler yields
    stream item values; this helper wraps them in canonical ``stream_item``
    frames and emits the single terminal frame.
    """

    serve_plugin(
        stream_handler=handler,
        input_stream=input_stream,
        output_stream=output_stream,
        stream_terminal_reason=terminal_reason,
    )


def serve_bidi_plugin(
    handler: SidecarBidiHandler,
    *,
    input_stream: TextIO | None = None,
    output_stream: TextIO | None = None,
    terminal_reason: str = "bidi_complete",
) -> None:
    """Run one declarative bidi plugin invocation.

    The runtime host writes one ``bidi_open`` request, then zero or more
    ``bidi_input`` frames, then ``close``. The handler receives a typed
    invocation plus an iterator over validated input frame payloads. Values
    yielded by the handler are wrapped as canonical ``bidi_output`` frames.
    """

    serve_plugin(
        bidi_handler=handler,
        input_stream=input_stream,
        output_stream=output_stream,
        bidi_terminal_reason=terminal_reason,
    )


def serve_plugin(
    *,
    invoke_handler: SidecarHandler | None = None,
    stream_handler: SidecarStreamHandler | None = None,
    bidi_handler: SidecarBidiHandler | None = None,
    input_stream: TextIO | None = None,
    output_stream: TextIO | None = None,
    stream_terminal_reason: str = "stream_complete",
    bidi_terminal_reason: str = "bidi_complete",
) -> None:
    """Dispatch one sidecar request frame to a provider-owned helper path."""

    input_stream = input_stream or sys.stdin
    output_stream = output_stream or sys.stdout
    call_id = ""
    try:
        frame = _read_frame(input_stream)
        call_id = _frame_call_id(frame)
        frame_type = _required_string(frame, "type")
        if frame_type == "invoke":
            if invoke_handler is None:
                raise SidecarProtocolError("invoke sidecar helper is not configured")
            invocation = SidecarInvocation.from_frame(frame, expected_type="invoke")
            value = invoke_handler(invocation)
            _write_frame(
                {"type": "result", "call_id": invocation.call_id, "value": value},
                output_stream,
            )
            return
        if frame_type == "stream_open":
            if stream_handler is None:
                raise SidecarProtocolError("stream sidecar helper is not configured")
            invocation = SidecarInvocation.from_frame(frame, expected_type="stream_open")
            for value in stream_handler(invocation):
                _write_frame(
                    {
                        "type": "stream_item",
                        "call_id": invocation.call_id,
                        "value": value,
                    },
                    output_stream,
                )
            _write_frame(
                {
                    "type": "terminal",
                    "call_id": invocation.call_id,
                    "reason": stream_terminal_reason,
                },
                output_stream,
            )
            return
        if frame_type == "bidi_open":
            if bidi_handler is None:
                raise SidecarProtocolError("bidi sidecar helper is not configured")
            invocation = SidecarInvocation.from_frame(frame, expected_type="bidi_open")
            input_frames = _iter_bidi_input_frames(input_stream, invocation.call_id)
            for value in bidi_handler(invocation, input_frames):
                _write_frame(
                    {
                        "type": "bidi_output",
                        "call_id": invocation.call_id,
                        "frame": value,
                    },
                    output_stream,
                )
            _write_frame(
                {
                    "type": "terminal",
                    "call_id": invocation.call_id,
                    "reason": bidi_terminal_reason,
                },
                output_stream,
            )
            return
        raise SidecarProtocolError(
            f"sidecar request type {frame_type!r} is not supported"
        )
    except Exception as exc:  # noqa: BLE001 - plugin boundary must become a frame.
        _write_frame(
            {"type": "error", "call_id": call_id, "message": str(exc)},
            output_stream,
        )


def _frame_call_id(frame: Mapping[str, Any]) -> str:
    value = frame.get("call_id")
    return value if isinstance(value, str) else ""


def _read_frame(input_stream: TextIO) -> Mapping[str, Any]:
    line = input_stream.readline()
    if not line:
        raise SidecarProtocolError("missing sidecar request frame")
    try:
        decoded = json.loads(line)
    except json.JSONDecodeError as exc:
        raise SidecarProtocolError(f"invalid sidecar request JSON: {exc}") from exc
    if not isinstance(decoded, Mapping):
        raise SidecarProtocolError("sidecar request frame must be an object")
    return decoded


def _write_frame(frame: Mapping[str, Any], output_stream: TextIO) -> None:
    output_stream.write(json.dumps(frame, separators=(",", ":")))
    output_stream.write("\n")
    output_stream.flush()


def _required_string(frame: Mapping[str, Any], field: str) -> str:
    value = frame.get(field)
    if not isinstance(value, str) or not value:
        raise SidecarProtocolError(f"sidecar frame field {field!r} must be a string")
    return value


def _reject_unknown_invocation_fields(frame: Mapping[str, Any]) -> None:
    for field in frame:
        if field not in _CANONICAL_INVOCATION_FIELDS:
            raise SidecarProtocolError(
                f"sidecar frame field {field!r} is not part of the canonical invocation frame"
            )


def _reject_unknown_open_request_fields(frame: Mapping[str, Any]) -> None:
    for field in frame:
        if field not in _CANONICAL_OPEN_REQUEST_FIELDS:
            raise SidecarProtocolError(
                f"sidecar request frame field {field!r} is not part of the canonical request frame"
            )


def _reject_unknown_fields(
    frame: Mapping[str, Any], allowed: frozenset[str], label: str
) -> None:
    for field in frame:
        if field not in allowed:
            raise SidecarProtocolError(
                f"{label} field {field!r} is not part of the canonical frame"
            )


def _require_invocation_fields(frame: Mapping[str, Any]) -> None:
    for field in (
        "caller_ura",
        "callee_ura",
        "ability_ura",
        "subject_ura",
        "invocation_nonce",
        "causal_context",
        "args",
    ):
        if field not in frame:
            raise SidecarProtocolError(f"sidecar frame field {field!r} is required")


def _required_mapping(frame: Mapping[str, Any], field: str) -> Mapping[str, Any]:
    value = frame.get(field)
    if not isinstance(value, Mapping):
        raise SidecarProtocolError(f"sidecar frame field {field!r} must be an object")
    return value


def _immutable_mapping(value: Mapping[str, Any]) -> Mapping[str, Any]:
    projected: dict[str, Any] = {}
    for key, item in value.items():
        if not isinstance(key, str):
            raise SidecarProtocolError("sidecar frame object keys must be strings")
        projected[key] = _immutable_value(item)
    return MappingProxyType(projected)


def _immutable_value(value: Any) -> Any:
    if isinstance(value, Mapping):
        return _immutable_mapping(value)
    if isinstance(value, (list, tuple)):
        return tuple(_immutable_value(item) for item in value)
    return value


def _required_nonce(frame: Mapping[str, Any], field: str) -> tuple[int, ...]:
    value = frame.get(field)
    if not isinstance(value, list) or not value:
        raise SidecarProtocolError(
            f"sidecar frame field {field!r} must be a byte array"
        )
    if len(value) != _CANONICAL_INVOCATION_NONCE_BYTES:
        raise SidecarProtocolError(
            f"sidecar frame field {field!r} must contain exactly "
            f"{_CANONICAL_INVOCATION_NONCE_BYTES} bytes"
        )
    nonce: list[int] = []
    for item in value:
        if isinstance(item, bool) or not isinstance(item, int) or item < 0 or item > 255:
            raise SidecarProtocolError(
                f"sidecar frame field {field!r} must contain bytes"
            )
        nonce.append(item)
    return tuple(nonce)


def _iter_bidi_input_frames(
    input_stream: TextIO, expected_call_id: str
) -> Iterator[Mapping[str, Any]]:
    while True:
        line = input_stream.readline()
        if not line:
            return
        if not line.strip():
            continue
        try:
            decoded = json.loads(line)
        except json.JSONDecodeError as exc:
            raise SidecarProtocolError(f"invalid sidecar request JSON: {exc}") from exc
        if not isinstance(decoded, Mapping):
            raise SidecarProtocolError("sidecar request frame must be an object")
        frame_type = _required_string(decoded, "type")
        call_id = _required_string(decoded, "call_id")
        if call_id != expected_call_id:
            raise SidecarProtocolError(
                f"sidecar frame call_id {call_id!r} does not match open call_id {expected_call_id!r}"
            )
        if frame_type == "bidi_input":
            _reject_unknown_fields(
                decoded,
                _CANONICAL_BIDI_INPUT_FIELDS,
                "sidecar bidi_input frame",
            )
            yield _immutable_mapping(_required_mapping(decoded, "frame"))
            continue
        if frame_type == "close":
            _reject_unknown_fields(decoded, _CANONICAL_CLOSE_FIELDS, "sidecar close frame")
            _required_string(decoded, "reason")
            return
        raise SidecarProtocolError(
            f"sidecar bidi request type {frame_type!r} is not supported"
        )


__all__ = [
    "SidecarHandler",
    "SidecarBidiHandler",
    "SidecarBidiInputFrames",
    "SidecarInvocation",
    "SidecarProtocolError",
    "SidecarStreamHandler",
    "serve_exec_plugin",
    "serve_bidi_plugin",
    "serve_plugin",
    "serve_stream_plugin",
]
