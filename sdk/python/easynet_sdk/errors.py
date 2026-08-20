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
    RUNTIME_OFFLINE = "RUNTIME_OFFLINE"
    PERMISSION_DENIED = "PERMISSION_DENIED"
    ADMISSION_DENIED = "ADMISSION_DENIED"
    HTTP_AUTH_DENIED = "HTTP_AUTH_DENIED"
    SIGNATURE_DENIED = "SIGNATURE_DENIED"
    POLICY_DENIED = "POLICY_DENIED"
    AUTHORITY_DENIED = "AUTHORITY_DENIED"
    ABILITY_NOT_FOUND = "ABILITY_NOT_FOUND"
    ROUTE_UNAVAILABLE = "ROUTE_UNAVAILABLE"
    EXECUTION_FAILED = "EXECUTION_FAILED"
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
    CALLER_IDENTITY_UNAVAILABLE = "CALLER_IDENTITY_UNAVAILABLE"
    CALLER_SIGNER_UNAVAILABLE = "CALLER_SIGNER_UNAVAILABLE"
    AUTHORITY_SUBJECT_MISMATCH = "AUTHORITY_SUBJECT_MISMATCH"
    DESCRIPTOR_NOT_FOUND = "DESCRIPTOR_NOT_FOUND"
    DESCRIPTOR_OWNER_OFFLINE = "DESCRIPTOR_OWNER_OFFLINE"
    DESCRIPTOR_MODE_UNSUPPORTED = "DESCRIPTOR_MODE_UNSUPPORTED"
    DESCRIPTOR_STALE = "DESCRIPTOR_STALE"
    RUNTIME_ROUTE_UNAVAILABLE = "RUNTIME_ROUTE_UNAVAILABLE"
    INVOCATION_CANCELLED = "INVOCATION_CANCELLED"
    INVOCATION_TIMEOUT = "INVOCATION_TIMEOUT"
    TERMINAL_RECEIPT_UNAVAILABLE = "TERMINAL_RECEIPT_UNAVAILABLE"
    RECEIPT_PROOF_FACTS_MISSING = "RECEIPT_PROOF_FACTS_MISSING"
    PROVIDER_UNAVAILABLE = "PROVIDER_UNAVAILABLE"


class ErrorClass(StrEnum):
    """Language-side grouping derived from canonical ErrorCode."""

    VALIDATION = "validation"
    HANDLE = "handle"
    LIFECYCLE = "lifecycle"
    AVAILABILITY = "availability"
    PERMISSION = "permission"
    ADMISSION = "admission"
    ROUTING = "routing"
    TIMEOUT = "timeout"
    CANCELLATION = "cancellation"
    PROTOCOL = "protocol"
    VERSION = "version"
    CONTROL = "control"
    UNSUPPORTED = "unsupported"
    GENERIC = "generic"


class RetryHint(StrEnum):
    """Retry classification for SDK errors."""

    NEVER = "never"
    SAFE = "safe"
    AFTER_BACKOFF = "after_backoff"
    UNKNOWN = "unknown"


RuntimeFailureCode = ErrorCode | str


@dataclass
class SDKError(Exception):
    """Typed error boundary used by Python SDK callers."""

    code: RuntimeFailureCode
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

    @property
    def error_class(self) -> ErrorClass:
        """Stable language-side grouping for this SDK error."""

        return error_class_for_code(self.code)

    @property
    def profile(self) -> str:
        """Profile detail attached to profile-originated SDK errors."""

        return _detail_string(self.details, "profile")

    @property
    def source_ref(self) -> str:
        """Stable package source reference attached to this SDK error."""

        return _detail_string(self.details, "source_ref")

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
                    message=f"decode runtime error JSON: {exc}",
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
                message=f"decode runtime error JSON: {exc}",
                cause=exc,
            ) from exc
        if not isinstance(decoded, dict):
            raise _invalid_runtime_error("runtime error JSON must be an object")

        code = _required_string(decoded, "code")
        stage = _required_string(decoded, "stage")
        message = _required_string(decoded, "message", allow_empty=True)
        retry = _retry_hint(_required_string(decoded, "retry"))
        details = decoded.get("details", {})
        if not isinstance(details, dict):
            raise _invalid_runtime_error("details must be an object")

        normalized_code = normalize_error_code(code)
        message = _canonical_runtime_error_message(normalized_code, message, details)
        return cls(
            code=normalized_code,
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


def error_class_for_code(code: ErrorCode | str) -> ErrorClass:
    """Project a canonical error code into a stable language class."""

    raw_code = code.value if isinstance(code, ErrorCode) else code
    try:
        normalized = normalize_error_code(raw_code)
    except SDKError:
        if _is_canonical_extension_error_code(raw_code):
            return ErrorClass.GENERIC
        raise
    match normalized:
        case (
            ErrorCode.INVALID_ARGUMENT
            | ErrorCode.NULL_POINTER
            | ErrorCode.INVALID_UTF8
            | ErrorCode.INVALID_INVOCATION
        ):
            return ErrorClass.VALIDATION
        case ErrorCode.INVALID_HANDLE:
            return ErrorClass.HANDLE
        case ErrorCode.NOT_INITIALIZED | ErrorCode.ALREADY_INIT:
            return ErrorClass.LIFECYCLE
        case ErrorCode.RUNTIME_OFFLINE | ErrorCode.TRANSPORT:
            return ErrorClass.AVAILABILITY
        case (
            ErrorCode.PERMISSION_DENIED
            | ErrorCode.HTTP_AUTH_DENIED
            | ErrorCode.CALLER_IDENTITY_UNAVAILABLE
        ):
            return ErrorClass.PERMISSION
        case (
            ErrorCode.ADMISSION_DENIED
            | ErrorCode.SIGNATURE_DENIED
            | ErrorCode.POLICY_DENIED
            | ErrorCode.AUTHORITY_DENIED
            | ErrorCode.AUTHORITY_SUBJECT_MISMATCH
            | ErrorCode.EXECUTION_FAILED
            | ErrorCode.ABILITY_FAILED
            | ErrorCode.CALLER_SIGNER_UNAVAILABLE
            | ErrorCode.RECEIPT_PROOF_FACTS_MISSING
        ):
            return ErrorClass.ADMISSION
        case (
            ErrorCode.ABILITY_NOT_FOUND
            | ErrorCode.ROUTE_UNAVAILABLE
            | ErrorCode.NOT_FOUND
            | ErrorCode.DESCRIPTOR_NOT_FOUND
            | ErrorCode.DESCRIPTOR_OWNER_OFFLINE
            | ErrorCode.DESCRIPTOR_MODE_UNSUPPORTED
            | ErrorCode.DESCRIPTOR_STALE
            | ErrorCode.RUNTIME_ROUTE_UNAVAILABLE
            | ErrorCode.PROVIDER_UNAVAILABLE
        ):
            return ErrorClass.ROUTING
        case ErrorCode.TIMEOUT | ErrorCode.INVOCATION_TIMEOUT:
            return ErrorClass.TIMEOUT
        case ErrorCode.CANCELLED | ErrorCode.INVOCATION_CANCELLED:
            return ErrorClass.CANCELLATION
        case ErrorCode.PROTOCOL_MISMATCH | ErrorCode.PROTOCOL:
            return ErrorClass.PROTOCOL
        case ErrorCode.VERSION_MISMATCH | ErrorCode.VERSION_INCOMPATIBLE:
            return ErrorClass.VERSION
        case ErrorCode.CONTROL_ONLY:
            return ErrorClass.CONTROL
        case ErrorCode.NOT_IMPLEMENTED:
            return ErrorClass.UNSUPPORTED
        case _:
            return ErrorClass.GENERIC


def normalize_error_code(code: str) -> ErrorCode:
    """Parse the current canonical runtime error-code schema value."""

    try:
        return ErrorCode(code)
    except ValueError as exc:
        raise _invalid_runtime_error(f"unknown runtime error code: {code}") from exc


def canonical_failure_code(code: str | None = None) -> RuntimeFailureCode:
    """Project runtime/wire failure codes into the canonical SDK taxonomy."""

    if code is not None:
        code = code.strip()
    if code:
        try:
            return normalize_error_code(code)
        except SDKError:
            if _is_canonical_extension_error_code(code):
                return code
            return ErrorCode.PROTOCOL_MISMATCH
    return ErrorCode.PROTOCOL_MISMATCH


def _canonical_runtime_error_message(
    code: ErrorCode, message: str, details: Mapping[str, object]
) -> str:
    if code == ErrorCode.DESCRIPTOR_OWNER_OFFLINE:
        return "DESCRIPTOR_OWNER_OFFLINE: descriptor owner is not online"
    if code != ErrorCode.CALLER_SIGNER_UNAVAILABLE:
        return message
    caller_ura = _caller_ura_from_signer_error_message(message) or _detail_string(
        details, "caller_ura"
    )
    if caller_ura.strip():
        return (
            "CALLER_SIGNER_UNAVAILABLE: remote invocation requires a caller signer "
            f"for `{caller_ura.strip()}`; load or provision that identity in the "
            "local key service"
        )
    return (
        "CALLER_SIGNER_UNAVAILABLE: remote invocation requires a caller signer; "
        "load or provision that identity in the local key service"
    )


def _caller_ura_from_signer_error_message(message: str) -> str:
    marker = "for `"
    marker_index = message.find(marker)
    if marker_index < 0:
        return ""
    tail = message[marker_index + len(marker) :]
    end_index = tail.find("`")
    if end_index < 0:
        return ""
    return tail[:end_index].strip()


def canonical_terminal_state_code(state: str) -> ErrorCode:
    """Project terminal runtime states into canonical SDK error codes."""

    match state:
        case "TimedOut":
            return ErrorCode.TIMEOUT
        case "Cancelled":
            return ErrorCode.CANCELLED
        case "Failed":
            return ErrorCode.ADMISSION_DENIED
        case _:
            return ErrorCode.PROTOCOL_MISMATCH


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
    value.setdefault("source_ref", source_ref or profile_source_ref(profile))
    for key, ref_value in refs.items():
        if ref_value is not None and ref_value != "":
            value.setdefault(key, ref_value)
    return value


def profile_source_ref(profile: str) -> str:
    """Return the stable Python package source reference for a profile."""

    clean = profile.strip()
    if not clean:
        return ""
    return f"python_sdk.profile.{clean}"


def _retry_hint(value: str) -> RetryHint:
    try:
        return RetryHint(value)
    except ValueError as exc:
        raise _invalid_runtime_error(
            "retry must be never, safe, after_backoff, or unknown"
        ) from exc


def _required_string(
    decoded: Mapping[str, object], field_name: str, *, allow_empty: bool = False
) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or (not allow_empty and value == ""):
        raise _invalid_runtime_error(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_runtime_error(f"{field_name} must be a string or null")
    return value


def _invalid_runtime_error(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="decode",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )


def _detail_string(details: Mapping[str, object], key: str) -> str:
    value = details.get(key)
    return value if isinstance(value, str) else ""


def _is_canonical_extension_error_code(code: str) -> bool:
    if code in {"DAEMON_DOWN", "DAEMON_OFFLINE"}:
        return False
    saw_letter = False
    for char in code:
        if "A" <= char <= "Z":
            saw_letter = True
            continue
        if char == "_" or "0" <= char <= "9":
            continue
        return False
    return saw_letter
