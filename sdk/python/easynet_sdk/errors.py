"""Typed Python SDK errors."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Mapping, Optional


class ErrorCode(StrEnum):
    """Stable Python SDK error classification."""

    INVALID_ARGUMENT = "INVALID_ARGUMENT"
    INVALID_HANDLE = "INVALID_HANDLE"
    NULL_POINTER = "NULL_POINTER"
    INVALID_UTF8 = "INVALID_UTF8"
    NOT_INITIALIZED = "NOT_INITIALIZED"
    ALREADY_INIT = "ALREADY_INIT"
    DAEMON_OFFLINE = "DAEMON_OFFLINE"
    PERMISSION_DENIED = "PERMISSION_DENIED"
    ADMISSION_DENIED = "ADMISSION_DENIED"
    ABILITY_NOT_FOUND = "ABILITY_NOT_FOUND"
    ROUTE_UNAVAILABLE = "ROUTE_UNAVAILABLE"
    TIMEOUT = "TIMEOUT"
    CANCELLED = "CANCELLED"
    INVALID_INVOCATION = "INVALID_INVOCATION"
    PROTOCOL_MISMATCH = "PROTOCOL_MISMATCH"
    VERSION_MISMATCH = "VERSION_MISMATCH"
    VERSION_INCOMPATIBLE = "VERSION_INCOMPATIBLE"
    CONTROL_ONLY = "CONTROL_ONLY"
    TRANSPORT = "TRANSPORT"
    PROTOCOL = "PROTOCOL"
    NOT_FOUND = "NOT_FOUND"
    ABILITY_FAILED = "ABILITY_FAILED"
    NOT_IMPLEMENTED = "NOT_IMPLEMENTED"
    GENERIC = "GENERIC"


class RetryHint(StrEnum):
    """Retry classification for SDK errors."""

    NEVER = "never"
    SAFE = "safe"
    AFTER_BACKOFF = "after_backoff"
    UNKNOWN = "unknown"


@dataclass(frozen=True)
class SDKError(Exception):
    """Typed error boundary used by Python SDK callers."""

    code: ErrorCode
    stage: str
    retry: RetryHint
    message: str
    retryable: bool = False
    source: Optional[str] = None
    invocation_id: Optional[str] = None
    receipt_ura: Optional[str] = None
    details: Mapping[str, object] = field(default_factory=dict)
    cause: Optional[BaseException] = None

    def __post_init__(self) -> None:
        Exception.__init__(self, self.message)

    def __str__(self) -> str:
        return f"{self.code}: {self.message}" if self.message else str(self.code)

    @classmethod
    def from_json(cls, raw: bytes | str) -> Optional["SDKError"]:
        """Decode the shared error.schema.json DTO into an SDKError."""

        if isinstance(raw, bytes):
            try:
                text = raw.decode("utf-8")
            except UnicodeDecodeError as exc:
                raise SDKError(
                    code=ErrorCode.INVALID_ARGUMENT,
                    stage="decode",
                    retry=RetryHint.NEVER,
                    retryable=False,
                    message=f"decode daemon error JSON: {exc}",
                    cause=exc,
                ) from exc
        else:
            text = raw
        if text.strip() == "null":
            return None
        try:
            decoded = json.loads(text)
        except Exception as exc:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="decode",
                retry=RetryHint.NEVER,
                retryable=False,
                message=f"decode daemon error JSON: {exc}",
                cause=exc,
            ) from exc
        if not isinstance(decoded, dict):
            raise _invalid_daemon_error("daemon error JSON must be an object")

        code = _required_string(decoded, "code")
        stage = _required_string(decoded, "stage")
        message = _required_string(decoded, "message", allow_empty=True)
        retry = _retry_hint(_required_string(decoded, "retry"))
        details = decoded.get("details", {})
        if not isinstance(details, dict):
            raise _invalid_daemon_error("details must be an object")

        return cls(
            code=normalize_error_code(code),
            stage=stage,
            retry=retry,
            retryable=retryable_for_hint(retry),
            message=message,
            source=_optional_string(decoded.get("source"), "source"),
            invocation_id=_optional_string(decoded.get("invocation_id"), "invocation_id"),
            receipt_ura=_optional_string(decoded.get("receipt_ura"), "receipt_ura"),
            details=dict(details),
        )


def is_code(error: BaseException, code: ErrorCode) -> bool:
    """Return whether *error* is an SDKError with the requested code."""

    return isinstance(error, SDKError) and error.code == code


RuntimeError = SDKError


def retryable_for_hint(retry: RetryHint) -> bool:
    """Return the explicit retryability represented by a retry hint."""

    return retry in {RetryHint.SAFE, RetryHint.AFTER_BACKOFF}


def normalize_error_code(code: str) -> ErrorCode:
    """Map daemon/C ABI wire codes into the SDK taxonomy."""

    aliases = {
        "InvalidArgument": ErrorCode.INVALID_ARGUMENT,
        "INVALID_ARGUMENT": ErrorCode.INVALID_ARGUMENT,
        "InvalidHandle": ErrorCode.INVALID_HANDLE,
        "INVALID_HANDLE": ErrorCode.INVALID_HANDLE,
        "NullPointer": ErrorCode.NULL_POINTER,
        "NULL_POINTER": ErrorCode.NULL_POINTER,
        "InvalidUTF8": ErrorCode.INVALID_UTF8,
        "INVALID_UTF8": ErrorCode.INVALID_UTF8,
        "NotInitialized": ErrorCode.NOT_INITIALIZED,
        "NOT_INITIALIZED": ErrorCode.NOT_INITIALIZED,
        "AlreadyInit": ErrorCode.ALREADY_INIT,
        "ALREADY_INIT": ErrorCode.ALREADY_INIT,
        "DaemonDown": ErrorCode.DAEMON_OFFLINE,
        "DAEMON_DOWN": ErrorCode.DAEMON_OFFLINE,
        "DAEMON_OFFLINE": ErrorCode.DAEMON_OFFLINE,
        "PermissionDenied": ErrorCode.PERMISSION_DENIED,
        "PERMISSION_DENIED": ErrorCode.PERMISSION_DENIED,
        "AdmissionDenied": ErrorCode.ADMISSION_DENIED,
        "ADMISSION_DENIED": ErrorCode.ADMISSION_DENIED,
        "AbilityNotFound": ErrorCode.ABILITY_NOT_FOUND,
        "ABILITY_NOT_FOUND": ErrorCode.ABILITY_NOT_FOUND,
        "RouteUnavailable": ErrorCode.ROUTE_UNAVAILABLE,
        "ROUTE_UNAVAILABLE": ErrorCode.ROUTE_UNAVAILABLE,
        "Timeout": ErrorCode.TIMEOUT,
        "TIMEOUT": ErrorCode.TIMEOUT,
        "Cancelled": ErrorCode.CANCELLED,
        "CANCELLED": ErrorCode.CANCELLED,
        "InvalidInvocation": ErrorCode.INVALID_INVOCATION,
        "INVALID_INVOCATION": ErrorCode.INVALID_INVOCATION,
        "ProtocolMismatch": ErrorCode.PROTOCOL_MISMATCH,
        "PROTOCOL_MISMATCH": ErrorCode.PROTOCOL_MISMATCH,
        "VersionMismatch": ErrorCode.VERSION_MISMATCH,
        "VERSION_MISMATCH": ErrorCode.VERSION_MISMATCH,
        "VersionIncompatible": ErrorCode.VERSION_INCOMPATIBLE,
        "VERSION_INCOMPATIBLE": ErrorCode.VERSION_INCOMPATIBLE,
        "ControlOnly": ErrorCode.CONTROL_ONLY,
        "CONTROL_ONLY": ErrorCode.CONTROL_ONLY,
        "Transport": ErrorCode.TRANSPORT,
        "TRANSPORT": ErrorCode.TRANSPORT,
        "Protocol": ErrorCode.PROTOCOL,
        "PROTOCOL": ErrorCode.PROTOCOL,
        "NotFound": ErrorCode.NOT_FOUND,
        "NOT_FOUND": ErrorCode.NOT_FOUND,
        "AbilityFailed": ErrorCode.ABILITY_FAILED,
        "ABILITY_FAILED": ErrorCode.ABILITY_FAILED,
        "NotImplemented": ErrorCode.NOT_IMPLEMENTED,
        "NOT_IMPLEMENTED": ErrorCode.NOT_IMPLEMENTED,
        "Generic": ErrorCode.GENERIC,
        "GENERIC": ErrorCode.GENERIC,
    }
    if code not in aliases:
        raise _invalid_daemon_error(f"unknown daemon error code: {code}")
    return aliases[code]


def profile_error_details(
    profile: str,
    *,
    source_ref: str = "",
    details: Mapping[str, object] | None = None,
    **refs: object,
) -> dict[str, object]:
    """Return stable profile-origin metadata for SDKError.details."""

    value = dict(details or {})
    value.setdefault("profile", profile)
    value.setdefault("source_ref", source_ref or f"python_sdk.profile.{profile}")
    for key, ref_value in refs.items():
        if ref_value is not None and ref_value != "":
            value.setdefault(key, ref_value)
    return value


def _retry_hint(value: str) -> RetryHint:
    try:
        return RetryHint(value)
    except ValueError as exc:
        raise _invalid_daemon_error(
            "retry must be never, safe, after_backoff, or unknown"
        ) from exc


def _required_string(
    decoded: Mapping[str, object], field_name: str, *, allow_empty: bool = False
) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or (not allow_empty and value == ""):
        raise _invalid_daemon_error(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_daemon_error(f"{field_name} must be a string or null")
    return value


def _invalid_daemon_error(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="decode",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )
