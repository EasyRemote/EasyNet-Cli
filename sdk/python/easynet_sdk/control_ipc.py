"""Direct daemon control-plane IPC facade.

This module speaks only the daemon boot/status control socket protocol. It is
not an Invocation transport and must not be used for product ability calls.
"""

from __future__ import annotations

import json
import socket
import struct
from dataclasses import dataclass, field
from pathlib import Path
from typing import Mapping, Optional

from .errors import ErrorCode, RetryHint, SDKError


_CONTROL_IPC_VERSION = 1
_MAX_CONTROL_FRAME_BYTES = 8 * 1024 * 1024
_CONTROL_BOOT_STATUS_ABILITY = "system.watch_boot"
_CONTROL_FRAME_TYPES = {"subscribe", "cancel"}


@dataclass(frozen=True)
class _IpcVersionRange:
    """Inclusive control IPC version range."""

    min: int
    max: int

    @classmethod
    def from_mapping(cls, value: object) -> "_IpcVersionRange":
        if not isinstance(value, Mapping):
            raise _invalid_control("supported_ipc_versions must be an object")
        minimum = _required_non_negative_int(value, "min")
        maximum = _required_non_negative_int(value, "max")
        if minimum <= 0 or maximum <= 0 or minimum > maximum:
            raise _invalid_control("supported_ipc_versions is invalid")
        return cls(min=minimum, max=maximum)

    def overlap(self, other: "_IpcVersionRange") -> Optional["_IpcVersionRange"]:
        minimum = max(self.min, other.min)
        maximum = min(self.max, other.max)
        if minimum > maximum:
            return None
        return _IpcVersionRange(min=minimum, max=maximum)


@dataclass(frozen=True)
class _ControlDiscovery:
    """Parsed control discovery file."""

    socket_path: str = ""
    pipe_name: str = ""
    invocation_endpoint: str = ""
    pid: int = 0
    daemon_version: str = ""
    supported_ipc_versions: _IpcVersionRange = field(
        default_factory=lambda: _IpcVersionRange(_CONTROL_IPC_VERSION, _CONTROL_IPC_VERSION)
    )
    capability_flags: tuple[str, ...] = ()
    pages_port: int = 0

    @classmethod
    def from_json(cls, raw: bytes | str) -> "_ControlDiscovery":
        decoded = _json_object(raw, "control discovery")
        flags = decoded.get("capability_flags", [])
        if not isinstance(flags, list) or not all(isinstance(item, str) for item in flags):
            raise _invalid_control("capability_flags must be an array of strings")
        return cls(
            socket_path=_optional_string(decoded.get("socket_path"), "socket_path") or "",
            pipe_name=_optional_string(decoded.get("pipe_name"), "pipe_name") or "",
            invocation_endpoint=_optional_string(
                decoded.get("invocation_endpoint"), "invocation_endpoint"
            )
            or "",
            pid=_optional_non_negative_int(decoded.get("pid"), "pid"),
            daemon_version=_optional_string(decoded.get("daemon_version"), "daemon_version")
            or "",
            supported_ipc_versions=_IpcVersionRange.from_mapping(
                decoded.get("supported_ipc_versions")
            ),
            capability_flags=tuple(flags),
            pages_port=_optional_non_negative_int(decoded.get("pages_port"), "pages_port"),
        )


@dataclass(frozen=True)
class _ControlFrame:
    """One daemon control-plane response frame."""

    frame_type: str
    subscription_id: str = ""
    frame: object = None
    reason: str = ""
    code: str = ""
    message: str = ""

    @classmethod
    def from_json(cls, raw: bytes | str) -> "_ControlFrame":
        decoded = _json_object(raw, "control frame")
        frame_type = _required_string(decoded, "type")
        if frame_type == "frame":
            return cls(
                frame_type=frame_type,
                subscription_id=_required_string(decoded, "subscription_id"),
                frame=decoded.get("frame"),
            )
        if frame_type == "terminal":
            return cls(
                frame_type=frame_type,
                subscription_id=_required_string(decoded, "subscription_id"),
                reason=_required_string(decoded, "reason"),
            )
        if frame_type == "error":
            return cls(
                frame_type=frame_type,
                subscription_id=_optional_string(
                    decoded.get("subscription_id"), "subscription_id"
                )
                or "",
                code=_required_string(decoded, "code"),
                message=_required_string(decoded, "message", allow_empty=True),
            )
        raise _invalid_control("unknown control frame type")

    def is_error(self) -> bool:
        return self.frame_type == "error"


class _ControlIpcClient:
    """Length-prefixed JSON client for daemon boot/status control frames."""

    def __init__(
        self,
        sock: socket.socket,
        *,
        discovery: _ControlDiscovery,
        ipc_version: int,
        max_frame_bytes: int = _MAX_CONTROL_FRAME_BYTES,
    ) -> None:
        if max_frame_bytes <= 0:
            raise _invalid_control("max_frame_bytes must be positive")
        self._sock = sock
        self.discovery = discovery
        self.ipc_version = ipc_version
        self.max_frame_bytes = max_frame_bytes
        self._closed = False

    @classmethod
    def connect(
        cls,
        control_path: str | Path = "",
        *,
        timeout: float | None = None,
        max_frame_bytes: int = _MAX_CONTROL_FRAME_BYTES,
    ) -> "_ControlIpcClient":
        discovery = _read_control_discovery(control_path or _default_control_path())
        chosen = _negotiate_ipc_version(discovery.supported_ipc_versions)
        if not discovery.socket_path:
            raise SDKError(
                code=ErrorCode.NOT_IMPLEMENTED,
                stage="control_ipc",
                retry=RetryHint.NEVER,
                retryable=False,
                message="control discovery did not advertise a Unix socket path",
            )
        try:
            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            if timeout is not None:
                sock.settimeout(timeout)
            sock.connect(discovery.socket_path)
        except SDKError:
            raise
        except Exception as exc:
            raise SDKError(
                code=ErrorCode.RUNTIME_OFFLINE,
                stage="control_ipc",
                retry=RetryHint.SAFE,
                retryable=True,
                message="connect daemon control socket failed",
                cause=exc,
            ) from exc
        return cls(
            sock,
            discovery=discovery,
            ipc_version=chosen,
            max_frame_bytes=max_frame_bytes,
        )

    def subscribe(
        self,
        subscription_id: str,
        ability: str,
        args: Mapping[str, object] | None = None,
    ) -> None:
        self.send(
            {
                "type": "subscribe",
                "subscription_id": _clean_string(subscription_id, "subscription_id"),
                "ability": _clean_string(ability, "ability"),
                "args": dict(args or {}),
            }
        )

    def cancel(self, subscription_id: str) -> None:
        self.send(
            {
                "type": "cancel",
                "subscription_id": _clean_string(subscription_id, "subscription_id"),
            }
        )

    def round_trip(self, frame: Mapping[str, object]) -> _ControlFrame:
        self.send(frame)
        return self.recv()

    def send(self, frame: Mapping[str, object]) -> None:
        self._require_open()
        raw = _json_bytes(_validate_outgoing_control_frame(frame))
        if len(raw) > self.max_frame_bytes:
            raise _invalid_control("control frame exceeds max_frame_bytes")
        packet = struct.pack("<I", len(raw)) + raw
        try:
            self._sock.sendall(packet)
        except Exception as exc:
            raise SDKError(
                code=ErrorCode.ROUTE_UNAVAILABLE,
                stage="control_ipc",
                retry=RetryHint.SAFE,
                retryable=True,
                message="send control frame failed",
                cause=exc,
            ) from exc

    def recv(self) -> _ControlFrame:
        self._require_open()
        header = self._recv_exact(4)
        size = struct.unpack("<I", header)[0]
        if size <= 0 or size > self.max_frame_bytes:
            raise _invalid_control("control frame size is invalid")
        return _ControlFrame.from_json(self._recv_exact(size))

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        self._sock.close()

    def __enter__(self) -> "_ControlIpcClient":
        self._require_open()
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()

    def _recv_exact(self, size: int) -> bytes:
        chunks: list[bytes] = []
        remaining = size
        try:
            while remaining > 0:
                chunk = self._sock.recv(remaining)
                if not chunk:
                    raise SDKError(
                        code=ErrorCode.RUNTIME_OFFLINE,
                        stage="control_ipc",
                        retry=RetryHint.SAFE,
                        retryable=True,
                        message="daemon control socket closed before a full frame arrived",
                    )
                chunks.append(chunk)
                remaining -= len(chunk)
        except SDKError:
            raise
        except Exception as exc:
            raise SDKError(
                code=ErrorCode.ROUTE_UNAVAILABLE,
                stage="control_ipc",
                retry=RetryHint.SAFE,
                retryable=True,
                message="read control frame failed",
                cause=exc,
            ) from exc
        return b"".join(chunks)

    def _require_open(self) -> None:
        if self._closed:
            raise SDKError(
                code=ErrorCode.CANCELLED,
                stage="control_ipc",
                retry=RetryHint.NEVER,
                retryable=False,
                message="control IPC client is closed",
            )


def _default_control_path() -> Path:
    """Return the default daemon control discovery file path."""

    return Path.home() / ".easynet" / "control.json"


def _read_control_discovery(control_path: str | Path = "") -> _ControlDiscovery:
    path = Path(control_path or _default_control_path())
    try:
        raw = path.read_bytes()
    except FileNotFoundError as exc:
        raise SDKError(
            code=ErrorCode.RUNTIME_OFFLINE,
            stage="control_ipc",
            retry=RetryHint.SAFE,
            retryable=True,
            message=f"control discovery file not found at {path}",
            cause=exc,
        ) from exc
    except Exception as exc:
        raise SDKError(
            code=ErrorCode.ROUTE_UNAVAILABLE,
            stage="control_ipc",
            retry=RetryHint.SAFE,
            retryable=True,
            message=f"read control discovery file failed at {path}",
            cause=exc,
        ) from exc
    return _ControlDiscovery.from_json(raw)


def _negotiate_ipc_version(daemon_range: _IpcVersionRange) -> int:
    supported = _IpcVersionRange(_CONTROL_IPC_VERSION, _CONTROL_IPC_VERSION)
    overlap = supported.overlap(daemon_range)
    if overlap is None:
        raise SDKError(
            code=ErrorCode.VERSION_MISMATCH,
            stage="control_ipc",
            retry=RetryHint.NEVER,
            retryable=False,
            message="daemon control IPC version is not compatible",
            details={
                "client_min": supported.min,
                "client_max": supported.max,
                "daemon_min": daemon_range.min,
                "daemon_max": daemon_range.max,
            },
        )
    return overlap.max


def _validate_outgoing_control_frame(frame: Mapping[str, object]) -> dict[str, object]:
    frame_type = _required_string(frame, "type")
    if frame_type not in _CONTROL_FRAME_TYPES:
        raise _invalid_control(
            "control IPC only accepts boot/status subscribe or cancel frames"
        )
    if frame_type == "cancel":
        return {
            "type": "cancel",
            "subscription_id": _clean_string(
                frame.get("subscription_id"), "subscription_id"
            ),
        }

    ability = _clean_string(frame.get("ability"), "ability")
    if ability != _CONTROL_BOOT_STATUS_ABILITY:
        raise _invalid_control(
            "control IPC subscriptions are limited to system.watch_boot"
        )
    args = frame.get("args", {})
    if args is None:
        args = {}
    if not isinstance(args, Mapping):
        raise _invalid_control("control subscription args must be an object")
    return {
        "type": "subscribe",
        "subscription_id": _clean_string(
            frame.get("subscription_id"), "subscription_id"
        ),
        "ability": ability,
        "args": dict(args),
    }


def _json_object(raw: bytes | str, label: str) -> dict[str, object]:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_control(f"decode {label} JSON failed: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_control(f"{label} JSON must be an object")
    return decoded


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _required_non_negative_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_control(f"{field_name} must be a non-negative integer")
    return value


def _optional_non_negative_int(value: object, field_name: str) -> int:
    if value is None:
        return 0
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_control(f"{field_name} must be a non-negative integer")
    return value


def _required_string(
    decoded: Mapping[str, object],
    field_name: str,
    *,
    allow_empty: bool = False,
) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or (not allow_empty and value.strip() == ""):
        raise _invalid_control(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_control(f"{field_name} must be a string or null")
    return value


def _clean_string(value: object, field_name: str) -> str:
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_control(f"{field_name} is required")
    if value != value.strip():
        raise _invalid_control(f"{field_name} must not contain surrounding whitespace")
    return value


def _invalid_control(
    message: str, cause: BaseException | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="control_ipc",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )
