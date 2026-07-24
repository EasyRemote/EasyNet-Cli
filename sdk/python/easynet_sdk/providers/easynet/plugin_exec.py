"""Provider-scoped helper for EasyNet-Cli declarative exec plugins.

This module owns the JSON frame details used between `easynet-daemon` and a
process-backed plugin. Plugin authors should implement a handler over
`SidecarInvocation`; they should not hand-write sidecar protocol frames.
"""

from __future__ import annotations

from collections.abc import Callable, Mapping
from dataclasses import dataclass
import json
import sys
from typing import Any, TextIO


class SidecarProtocolError(ValueError):
    """Raised when a daemon/plugin sidecar frame is structurally invalid."""


@dataclass(frozen=True)
class SidecarInvocation:
    """Typed view of one daemon-admitted plugin invocation."""

    call_id: str
    caller_ura: str
    callee_ura: str
    ability_ura: str
    subject_ura: str
    invocation_nonce: tuple[int, ...]
    causal_context: Any
    args: Mapping[str, Any]
    frame_type: str = "invoke"

    @classmethod
    def from_frame(cls, frame: Mapping[str, Any]) -> "SidecarInvocation":
        """Project a daemon sidecar frame into a handler-facing invocation."""

        frame_type = _required_string(frame, "type")
        if frame_type != "invoke":
            raise SidecarProtocolError(
                f"exec plugin expected invoke frame, got {frame_type!r}"
            )
        call_id = _required_string(frame, "call_id")
        invocation = _required_mapping(frame, "invocation")
        _reject_legacy_tuple_aliases(invocation)
        nonce = _required_nonce(invocation, "invocation_nonce")
        args = _optional_mapping(invocation.get("args"), "args")
        return cls(
            call_id=call_id,
            caller_ura=_required_string(invocation, "caller_ura"),
            callee_ura=_required_string(invocation, "callee_ura"),
            ability_ura=_required_string(invocation, "ability_ura"),
            subject_ura=_required_string(invocation, "subject_ura"),
            invocation_nonce=nonce,
            causal_context=invocation.get("causal_context"),
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

    The daemon writes one request frame to stdin and expects exactly one
    response frame on stdout. Handler exceptions are converted into daemon
    protocol `error` frames so tracebacks never corrupt stdout framing.
    """

    input_stream = input_stream or sys.stdin
    output_stream = output_stream or sys.stdout
    call_id = ""
    try:
        frame = _read_frame(input_stream)
        call_id = _required_string(frame, "call_id")
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


def _reject_legacy_tuple_aliases(frame: Mapping[str, Any]) -> None:
    for legacy, canonical in (
        ("caller", "caller_ura"),
        ("callee", "callee_ura"),
        ("ability", "ability_ura"),
        ("subject", "subject_ura"),
    ):
        if legacy in frame:
            raise SidecarProtocolError(
                f"sidecar frame field {legacy!r} is retired; use {canonical!r}"
            )


def _required_mapping(frame: Mapping[str, Any], field: str) -> Mapping[str, Any]:
    value = frame.get(field)
    return _optional_mapping(value, field, required=True)


def _optional_mapping(
    value: Any,
    field: str,
    *,
    required: bool = False,
) -> Mapping[str, Any]:
    if value is None and not required:
        return {}
    if not isinstance(value, Mapping):
        raise SidecarProtocolError(f"sidecar frame field {field!r} must be an object")
    return value


def _required_nonce(frame: Mapping[str, Any], field: str) -> tuple[int, ...]:
    value = frame.get(field)
    if not isinstance(value, list) or not value:
        raise SidecarProtocolError(
            f"sidecar frame field {field!r} must be a byte array"
        )
    nonce: list[int] = []
    for item in value:
        if not isinstance(item, int) or item < 0 or item > 255:
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
