"""Private transport for the daemon-owned key service."""

from __future__ import annotations

import base64
import json
import math
import socket
import struct
import time
from dataclasses import dataclass
from typing import Any, Mapping

from ...errors import ErrorCode, RetryHint, SDKError

MAX_KEY_SERVICE_CANONICAL_BYTES = 64 * 1024 * 1024
MAX_KEY_SERVICE_FRAME_BYTES = 90 * 1024 * 1024
KEY_SERVICE_PROTOCOL_VERSION = 2

_PRIVATE_FIELD_TOKENS = (
    "seed",
    "private",
    "vault",
    "passphrase",
    "master_key",
    "ciphertext",
)


@dataclass(frozen=True)
class KeyServiceClient:
    """Length-prefixed JSON client for the daemon-local custody boundary."""

    socket_path: str = ""
    timeout_seconds: float = 10.0

    def __post_init__(self) -> None:
        if not isinstance(self.socket_path, str) or not self.socket_path.strip():
            raise invalid_key_service_input("daemon key-service endpoint is required")
        object.__setattr__(self, "socket_path", self.socket_path.strip())
        if (
            isinstance(self.timeout_seconds, bool)
            or not isinstance(self.timeout_seconds, (int, float))
            or not math.isfinite(self.timeout_seconds)
        ):
            raise invalid_key_service_input(
                "daemon key-service timeout must be a finite number"
            )
        if self.timeout_seconds <= 0:
            object.__setattr__(self, "timeout_seconds", 10.0)

    def call(self, request: Mapping[str, Any]) -> Mapping[str, Any]:
        encoded = json.dumps(request, separators=(",", ":")).encode("utf-8")
        if len(encoded) > MAX_KEY_SERVICE_FRAME_BYTES:
            raise invalid_key_service_input(
                "daemon key-service request exceeds frame limit"
            )
        deadline = time.monotonic() + self.timeout_seconds
        connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            try:
                _set_remaining_timeout(connection, deadline)
                connection.connect(self.socket_path)
            except OSError as exc:
                raise _daemon_offline(self.socket_path, exc) from exc

            try:
                _set_remaining_timeout(connection, deadline)
                connection.sendall(struct.pack(">I", len(encoded)) + encoded)
                length = struct.unpack(">I", _read_exact(connection, 4, deadline))[0]
                if length > MAX_KEY_SERVICE_FRAME_BYTES:
                    raise invalid_key_service_payload(
                        "daemon key-service response exceeds frame limit"
                    )
                response = json.loads(
                    _read_exact(connection, length, deadline).decode("utf-8")
                )
            except SDKError:
                raise
            except OSError as exc:
                raise _transport_failure(self.socket_path, exc) from exc
        except SDKError:
            raise
        except (TypeError, ValueError, UnicodeDecodeError) as exc:
            raise invalid_key_service_payload(
                f"decode daemon key-service response: {exc}", exc
            ) from exc
        finally:
            connection.close()

        if not isinstance(response, dict):
            raise invalid_key_service_payload(
                "daemon key-service response must be an object"
            )
        reject_private_response_fields(response)
        result = required_response_string(response, "result")
        if result != "error":
            return response
        require_response_shape(response, "error", required=("kind", "message"))
        kind = required_response_string(response, "kind")
        message = required_response_string(response, "message")
        raise key_service_rejection(kind, message)


def require_result(response: Mapping[str, Any], expected: str) -> None:
    result = required_response_string(response, "result")
    if result != expected:
        raise invalid_key_service_payload(
            f"daemon key-service response result is {result!r}, want {expected!r}"
        )


def require_response_shape(
    response: Mapping[str, Any],
    expected_result: str,
    *,
    required: tuple[str, ...] = (),
    optional: tuple[str, ...] = (),
) -> None:
    """Validate one closed key-service response variant."""

    require_result(response, expected_result)
    allowed = {"result", *required, *optional}
    unknown = sorted(set(response).difference(allowed))
    if unknown:
        raise invalid_key_service_payload(
            "daemon key-service response contains unknown fields: " + ", ".join(unknown)
        )
    missing = [field for field in required if field not in response]
    if missing:
        raise invalid_key_service_payload(
            "daemon key-service response is missing fields: " + ", ".join(missing)
        )


def required_response_string(response: Mapping[str, Any], field: str) -> str:
    value = response.get(field)
    if not isinstance(value, str) or not value:
        raise invalid_key_service_payload(
            f"daemon key-service response field {field} must be a non-empty string"
        )
    return value


def required_response_i64(response: Mapping[str, Any], field: str) -> int:
    value = response.get(field)
    if isinstance(value, bool) or not isinstance(value, int):
        raise invalid_key_service_payload(
            f"daemon key-service response field {field} must be an integer"
        )
    if value < -(1 << 63) or value > (1 << 63) - 1:
        raise invalid_key_service_payload(
            f"daemon key-service response field {field} must fit i64"
        )
    return value


def required_response_bool(response: Mapping[str, Any], field: str) -> bool:
    value = response.get(field)
    if not isinstance(value, bool):
        raise invalid_key_service_payload(
            f"daemon key-service response field {field} must be a boolean"
        )
    return value


def decode_base64_field(
    response: Mapping[str, Any], field: str, *, expected_len: int | None = None
) -> bytes:
    return decode_base64_value(response.get(field), field, expected_len=expected_len)


def decode_base64_value(
    value: Any, field: str, *, expected_len: int | None = None
) -> bytes:
    if not isinstance(value, str) or not value:
        raise invalid_key_service_payload(
            f"daemon key-service response field {field} is required"
        )
    try:
        decoded = base64.b64decode(value, validate=True)
    except Exception as exc:
        raise invalid_key_service_payload(f"{field} must be base64", exc) from exc
    if expected_len is not None and len(decoded) != expected_len:
        raise invalid_key_service_payload(f"{field} must be {expected_len} bytes")
    if base64.b64encode(decoded).decode("ascii") != value:
        raise invalid_key_service_payload(f"{field} must be canonical base64")
    return decoded


def reject_private_response_fields(value: Any, path: str = "response") -> None:
    """Fail closed if a daemon projection ever exposes custody material."""

    if isinstance(value, Mapping):
        for field, nested in value.items():
            if not isinstance(field, str):
                raise invalid_key_service_payload(
                    f"daemon key-service {path} field names must be strings"
                )
            normalized = field.lower()
            if any(token in normalized for token in _PRIVATE_FIELD_TOKENS):
                raise invalid_key_service_payload(
                    f"daemon key-service {path} contains forbidden private field {field}"
                )
            reject_private_response_fields(nested, f"{path}.{field}")
    elif isinstance(value, list):
        for index, nested in enumerate(value):
            reject_private_response_fields(nested, f"{path}[{index}]")


def invalid_key_service_input(message: str, cause: Exception | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="key_service",
        retry=RetryHint.NEVER,
        message=message,
        cause=cause,
    )


def invalid_key_service_payload(
    message: str, cause: Exception | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.PROTOCOL,
        stage="key_service",
        retry=RetryHint.NEVER,
        message=message,
        cause=cause,
    )


def key_service_rejection(kind: str, message: str) -> SDKError:
    retry = RetryHint.NEVER
    code = ErrorCode.PROTOCOL
    if kind == "not_found":
        code = ErrorCode.NOT_FOUND
    elif kind == "already_exists":
        code = ErrorCode.ALREADY_INIT
    elif kind in {"lifecycle", "policy"}:
        code = ErrorCode.POLICY_DENIED
    elif kind in {"io", "durability_uncertain", "fail_stopped"}:
        code = ErrorCode.EXECUTION_FAILED
    elif kind in {"crypto", "kdf"}:
        code = ErrorCode.EXECUTION_FAILED
    return SDKError(
        code=code,
        stage="key_service",
        retry=retry,
        retryable=retry == RetryHint.SAFE,
        message=f"daemon key service rejected request ({kind}): {message}",
        details={"kind": kind},
    )


def _read_exact(connection: socket.socket, count: int, deadline: float) -> bytes:
    data = bytearray()
    while len(data) < count:
        _set_remaining_timeout(connection, deadline)
        chunk = connection.recv(count - len(data))
        if not chunk:
            raise OSError("unexpected EOF from daemon key service")
        data.extend(chunk)
    return bytes(data)


def _set_remaining_timeout(connection: socket.socket, deadline: float) -> None:
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError("daemon key-service request deadline exceeded")
    try:
        connection.settimeout(remaining)
    except (OverflowError, ValueError) as exc:
        raise invalid_key_service_input(
            "daemon key-service timeout is outside the platform range", exc
        ) from exc


def _daemon_offline(path: str, cause: OSError) -> SDKError:
    return SDKError(
        code=ErrorCode.DAEMON_OFFLINE,
        stage="key_service",
        retry=RetryHint.SAFE,
        message=f"daemon key service unavailable at {path}: {cause}",
        retryable=True,
        cause=cause,
    )


def _transport_failure(path: str, cause: OSError) -> SDKError:
    return SDKError(
        code=ErrorCode.TRANSPORT,
        stage="key_service",
        retry=RetryHint.SAFE,
        message=f"daemon key-service transport failed at {path}: {cause}",
        retryable=True,
        cause=cause,
    )
