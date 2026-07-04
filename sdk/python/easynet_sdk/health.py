"""Runtime Core health facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Mapping, Protocol, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError, retryable_for_hint


@runtime_checkable
class HealthTransport(Protocol):
    """Narrow transport interface for Runtime Core health."""

    def runtime_health(self) -> bytes:
        """Return raw runtime health JSON bytes from a daemon SDK boundary."""


@dataclass(frozen=True)
class RuntimeHealth:
    """Language-neutral SDK health DTO."""

    api_ready: bool
    daemon_ready: bool
    invocation_ready: bool
    directory_ready: bool
    trust_ready: bool
    runtime_ready: bool
    version: str | None = None
    abi_version: int | None = None
    mismatch: Mapping[str, object] | None = None
    diagnostics: tuple[str, ...] = field(default_factory=tuple)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "RuntimeHealth":
        """Decode the shared health.schema.json DTO."""

        return _decode_runtime_health(raw)

    def api_alive(self) -> bool:
        """Return process/API liveness, not full runtime readiness."""

        return self.api_ready and self.daemon_ready

    def ready(self) -> bool:
        """Return full runtime readiness."""

        return self.runtime_ready


class HealthClient:
    """Python Runtime Core health facade."""

    def __init__(self, transport: HealthTransport) -> None:
        if transport is None:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="sdk",
                retry=RetryHint.NEVER,
                message="health transport is required",
        )
        self._transport = transport
        self._closed = False

    def runtime_health(self) -> RuntimeHealth:
        """Read and decode daemon runtime health."""

        self._require_open()
        try:
            raw = self._transport.runtime_health()
        except SDKError:
            raise
        except Exception as exc:
            raise SDKError(
                code=ErrorCode.TRANSPORT,
                stage="transport",
                retry=RetryHint.SAFE,
                retryable=retryable_for_hint(RetryHint.SAFE),
                message="runtime health transport failed",
                cause=exc,
            ) from exc
        return _decode_runtime_health(raw)

    def close(self) -> None:
        """Close the underlying health transport when it owns resources."""

        if self._closed:
            return
        self._closed = True
        close = getattr(self._transport, "close", None)
        if close is not None:
            close()

    def _require_open(self) -> None:
        if self._closed:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="sdk",
                retry=RetryHint.NEVER,
                retryable=False,
                message="health client is closed",
            )


def _decode_runtime_health(raw: bytes) -> RuntimeHealth:
    try:
        decoded = json.loads(raw)
    except Exception as exc:
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="decode",
            retry=RetryHint.NEVER,
            message=f"decode runtime health JSON: {exc}",
            cause=exc,
        ) from exc
    if not isinstance(decoded, dict):
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="decode",
            retry=RetryHint.NEVER,
            message="runtime health JSON must be an object",
        )

    return RuntimeHealth(
        api_ready=_required_bool(decoded, "api_ready"),
        daemon_ready=_required_bool(decoded, "daemon_ready"),
        invocation_ready=_required_bool(decoded, "invocation_ready"),
        directory_ready=_required_bool(decoded, "directory_ready"),
        trust_ready=_required_bool(decoded, "trust_ready"),
        runtime_ready=_required_bool(decoded, "runtime_ready"),
        version=_optional_string(decoded.get("version"), "version"),
        abi_version=_optional_non_negative_int(decoded.get("abi_version"), "abi_version"),
        mismatch=_optional_object(decoded.get("mismatch"), "mismatch"),
        diagnostics=_diagnostics(decoded.get("diagnostics", [])),
    )


def _required_bool(decoded: Mapping[str, object], field_name: str) -> bool:
    if field_name not in decoded:
        raise _invalid_health_field(field_name, "is required")
    value = decoded[field_name]
    if not isinstance(value, bool):
        raise _invalid_health_field(field_name, "must be a boolean")
    return value


def _optional_string(value: object, field_name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_health_field(field_name, "must be a string or null")
    return value


def _optional_non_negative_int(value: object, field_name: str) -> int | None:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_health_field(field_name, "must be a non-negative integer or null")
    return value


def _optional_object(value: object, field_name: str) -> Mapping[str, object] | None:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_health_field(field_name, "must be an object or null")
    return dict(value)


def _diagnostics(value: object) -> tuple[str, ...]:
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise _invalid_health_field("diagnostics", "must be an array of strings")
    return tuple(value)


def _invalid_health_field(field_name: str, message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="decode",
        retry=RetryHint.NEVER,
        message=f"{field_name} {message}",
    )
