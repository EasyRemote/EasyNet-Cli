"""Local runtime environment projection.

The SDK owns the local runtime state-root and paired identity projection so
downstream products do not parse daemon credentials independently.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping

from .control_ipc import default_control_path
from .errors import ErrorCode, RetryHint, SDKError

_CREDENTIALS_FILENAME = "credentials.json"


@dataclass(frozen=True)
class RuntimeIdentityProjection:
    """Public local runtime identity projection.

    This projection contains routable public facts only. It never includes
    private keys, key-service endpoints, signer handles or product account
    records.
    """

    realm: str
    device_id: str
    username: str = ""
    hub_endpoint: str = ""


def runtime_state_root(control_path: str | Path = "") -> Path:
    """Resolve the SDK-owned local runtime state directory."""

    path = Path(control_path) if control_path else default_control_path()
    return path.parent


def runtime_credentials_path(control_path: str | Path = "") -> Path:
    """Resolve the paired runtime identity projection path."""

    return runtime_state_root(control_path) / _CREDENTIALS_FILENAME


def read_runtime_identity_projection(
    credentials_path: str | Path = "",
    *,
    control_path: str | Path = "",
) -> RuntimeIdentityProjection:
    """Read and validate the paired runtime identity projection."""

    path = Path(credentials_path) if credentials_path else runtime_credentials_path(control_path)
    try:
        raw = path.read_text(encoding="utf-8")
    except FileNotFoundError as exc:
        raise SDKError(
            code=ErrorCode.DAEMON_OFFLINE,
            stage="runtime_environment",
            retry=RetryHint.SAFE,
            retryable=True,
            message=f"runtime identity projection not readable at {path}",
            details={"credentials_path": str(path)},
            cause=exc,
        ) from exc
    return runtime_identity_projection_from_json(raw)


def runtime_identity_projection_from_json(
    raw: bytes | str,
) -> RuntimeIdentityProjection:
    """Decode a credentials projection."""

    try:
        decoded = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise _invalid(f"decode runtime identity projection JSON: {exc}", exc) from exc
    if not isinstance(decoded, Mapping):
        raise _invalid("runtime identity projection must be a JSON object")
    realm = _projection_text(decoded, "realm")
    device_id = _projection_text(decoded, "device_id")
    if not realm:
        raise _invalid("runtime identity projection missing realm")
    if not device_id:
        raise _invalid("runtime identity projection missing device_id")
    return RuntimeIdentityProjection(
        realm=realm,
        device_id=device_id,
        username=_projection_text(decoded, "username"),
        hub_endpoint=_projection_text(decoded, "hub_endpoint"),
    )


def _projection_text(raw: Mapping[str, object], key: str) -> str:
    value = raw.get(key)
    if value is None:
        return ""
    return str(value).strip()


def _invalid(message: str, cause: Exception | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime_environment",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )
