"""Provider-scoped helper for runtime declarative exec plugins.

This module owns the JSON frame details used between the runtime host and a
process-backed plugin. Plugin authors should implement a handler over
`SidecarInvocation`; they should not hand-write sidecar protocol frames.
"""

from __future__ import annotations

from collections.abc import Callable, Mapping
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
_CANONICAL_REQUEST_FIELDS = frozenset({"type", "call_id", "invocation"})
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
    def from_frame(cls, frame: Mapping[str, Any]) -> "SidecarInvocation":
        """Project a runtime sidecar frame into a handler-facing invocation."""

        _reject_unknown_request_fields(frame)
        frame_type = _required_string(frame, "type")
        if frame_type != "invoke":
            raise SidecarProtocolError(
                f"exec plugin expected invoke frame, got {frame_type!r}"
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

    input_stream = input_stream or sys.stdin
    output_stream = output_stream or sys.stdout
    call_id = ""
    try:
        frame = _read_frame(input_stream)
        call_id = _frame_call_id(frame)
        invocation = SidecarInvocation.from_frame(frame)
        value = handler(invocation)
        _write_frame(
            {"type": "result", "call_id": invocation.call_id, "value": value},
            output_stream,
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


def _reject_unknown_request_fields(frame: Mapping[str, Any]) -> None:
    for field in frame:
        if field not in _CANONICAL_REQUEST_FIELDS:
            raise SidecarProtocolError(
                f"sidecar request frame field {field!r} is not part of the canonical request frame"
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


__all__ = [
    "SidecarHandler",
    "SidecarInvocation",
    "SidecarProtocolError",
    "serve_exec_plugin",
]
