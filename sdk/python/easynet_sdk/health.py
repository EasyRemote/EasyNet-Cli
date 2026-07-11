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

    def runtime_diagnostics(self) -> bytes:
        """Return raw runtime diagnostics JSON bytes from a daemon SDK boundary."""


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


@dataclass(frozen=True)
class DiagnosticCheck:
    """One named readiness check in a diagnostics report."""

    name: str
    ready: bool
    message: str | None = None


@dataclass(frozen=True)
class DiagnosticsReport:
    """Language-neutral SDK diagnostics DTO."""

    profile: str
    kind: str
    state: str
    ready: bool
    version: str
    abi_version: int
    control_endpoint: str
    invocation_endpoint: str | None
    checks: tuple[DiagnosticCheck, ...]
    diagnostics: tuple[str, ...] = field(default_factory=tuple)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "DiagnosticsReport":
        """Decode the shared diagnostics.schema.json DTO."""

        return _decode_diagnostics_report(raw)


class HealthClient:
    """Python Runtime Core health facade."""

    def __init__(
        self, transport: HealthTransport, *, owns_transport: bool = True
    ) -> None:
        if transport is None:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="sdk",
                retry=RetryHint.NEVER,
                message="health transport is required",
            )
        self._transport = transport
        self._owns_transport = owns_transport
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
                code=ErrorCode.ROUTE_UNAVAILABLE,
                stage="transport",
                retry=RetryHint.SAFE,
                retryable=retryable_for_hint(RetryHint.SAFE),
                message="runtime health transport failed",
                cause=exc,
            ) from exc
        return _decode_runtime_health(raw)

    def diagnostics(self) -> DiagnosticsReport:
        """Read and decode daemon runtime diagnostics."""

        self._require_open()
        transport = getattr(self._transport, "runtime_diagnostics", None)
        if transport is None:
            raise SDKError(
                code=ErrorCode.NOT_IMPLEMENTED,
                stage="transport",
                retry=RetryHint.NEVER,
                retryable=False,
                message="health diagnostics transport is not available",
            )
        try:
            raw = transport()
        except SDKError:
            raise
        except Exception as exc:
            raise SDKError(
                code=ErrorCode.ROUTE_UNAVAILABLE,
                stage="transport",
                retry=RetryHint.SAFE,
                retryable=retryable_for_hint(RetryHint.SAFE),
                message="runtime diagnostics transport failed",
                cause=exc,
            ) from exc
        return _decode_diagnostics_report(raw)

    def close(self) -> None:
        """Close the underlying health transport when it owns resources."""

        if self._closed:
            return
        self._closed = True
        close = getattr(self._transport, "close", None)
        if self._owns_transport and close is not None:
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


def _decode_diagnostics_report(raw: bytes | str) -> DiagnosticsReport:
    try:
        decoded = json.loads(raw)
    except Exception as exc:
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="decode",
            retry=RetryHint.NEVER,
            message=f"decode diagnostics JSON: {exc}",
            cause=exc,
        ) from exc
    if not isinstance(decoded, dict):
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="decode",
            retry=RetryHint.NEVER,
            message="diagnostics JSON must be an object",
        )
    profile = _required_string(decoded, "profile")
    if profile != "health":
        raise _invalid_health_field("profile", "must be health")
    kind = _required_string(decoded, "kind")
    if kind != "diagnostics_report":
        raise _invalid_health_field("kind", "must be diagnostics_report")
    checks_raw = decoded.get("checks")
    if not isinstance(checks_raw, list) or not checks_raw:
        raise _invalid_health_field("checks", "must be non-empty")
    return DiagnosticsReport(
        profile=profile,
        kind=kind,
        state=_required_string(decoded, "state"),
        ready=_required_bool(decoded, "ready"),
        version=_required_string(decoded, "version"),
        abi_version=_required_non_negative_int(decoded, "abi_version"),
        control_endpoint=_required_string(decoded, "control_endpoint"),
        invocation_endpoint=_optional_string(
            decoded.get("invocation_endpoint"), "invocation_endpoint"
        ),
        checks=tuple(_diagnostic_check(item) for item in checks_raw),
        diagnostics=_diagnostics(decoded.get("diagnostics", [])),
    )


def _diagnostic_check(value: object) -> DiagnosticCheck:
    if not isinstance(value, dict):
        raise _invalid_health_field("checks", "items must be objects")
    return DiagnosticCheck(
        name=_required_string(value, "name"),
        ready=_required_bool(value, "ready"),
        message=_optional_string(value.get("message"), "message"),
    )


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value == "":
        raise _invalid_health_field(field_name, "must be a non-empty string")
    return value


def _required_non_negative_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_health_field(field_name, "must be a non-negative integer")
    return value


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
