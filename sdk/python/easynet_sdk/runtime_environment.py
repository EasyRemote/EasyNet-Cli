"""Local runtime environment projection.

The SDK owns the local runtime state-root and paired identity projection so
downstream products do not parse runtime-host credentials independently.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping

from .axon_addressing import user_ura
from .errors import ErrorCode, RetryHint, SDKError
from .providers.runtime.control import _default_control_path

_CREDENTIALS_FILENAME = "credentials.json"
_RUNTIME_IDENTITY_PROJECTION_FIELDS = frozenset(
    {
        "realm",
        "runtime_instance_id",
        "principal",
        "principal_display_name",
        "control_plane_endpoint",
    }
)


@dataclass(frozen=True)
class RuntimeIdentityProjection:
    """Public local runtime identity projection.

    This projection contains routable public facts only. It never includes
    private keys, key-service endpoints, signer handles or product account
    records.
    """

    realm: str
    runtime_instance_id: str
    principal: str = ""
    principal_display_name: str = ""
    control_plane_endpoint: str = ""


@dataclass(frozen=True)
class RuntimeIpcVersionRange:
    """Inclusive runtime-host control IPC version range."""

    min: int
    max: int


@dataclass(frozen=True)
class RuntimeControlIdentityProjection:
    """Public runtime-host identity advertised by control discovery."""

    mode: str
    realm: str
    runtime_instance_id: str = ""


@dataclass(frozen=True)
class RuntimeControlDiscovery:
    """Public runtime-host control discovery projection."""

    socket_path: str = ""
    pipe_name: str = ""
    invocation_endpoint: str = ""
    runtime_host_identity: RuntimeControlIdentityProjection | None = None
    pid: int = 0
    runtime_host_version: str = ""
    supported_ipc_versions: RuntimeIpcVersionRange = RuntimeIpcVersionRange(1, 1)
    capability_flags: tuple[str, ...] = ()


def runtime_state_root(control_path: str | Path = "") -> Path:
    """Resolve the SDK-owned local runtime state directory."""

    path = Path(control_path) if control_path else _default_control_path()
    return path.parent


def runtime_credentials_path(control_path: str | Path = "") -> Path:
    """Resolve the paired runtime identity projection path."""

    return runtime_state_root(control_path) / _CREDENTIALS_FILENAME


def read_runtime_control_discovery(
    control_path: str | Path = "",
) -> RuntimeControlDiscovery:
    """Read and validate the runtime-host control discovery projection."""

    from .providers.runtime.control import _read_control_discovery

    discovery = _read_control_discovery(control_path)
    identity = None
    if discovery.runtime_host_identity is not None:
        identity = RuntimeControlIdentityProjection(
            mode=discovery.runtime_host_identity.mode,
            realm=discovery.runtime_host_identity.realm,
            runtime_instance_id=discovery.runtime_host_identity.runtime_instance_id,
        )
    return RuntimeControlDiscovery(
        socket_path=discovery.socket_path,
        pipe_name=discovery.pipe_name,
        invocation_endpoint=discovery.invocation_endpoint,
        runtime_host_identity=identity,
        pid=discovery.pid,
        runtime_host_version=discovery.runtime_host_version,
        supported_ipc_versions=RuntimeIpcVersionRange(
            discovery.supported_ipc_versions.min,
            discovery.supported_ipc_versions.max,
        ),
        capability_flags=tuple(discovery.capability_flags),
    )


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
            code=ErrorCode.RUNTIME_OFFLINE,
            stage="runtime_environment",
            retry=RetryHint.SAFE,
            retryable=True,
            message=f"runtime identity projection not readable at {path}",
            details={"credentials_path": str(path)},
            cause=exc,
        ) from exc
    return runtime_identity_projection_from_json(raw)


def read_paired_runtime_identity_projection(
    credentials_path: str | Path = "",
    *,
    control_path: str | Path = "",
) -> RuntimeIdentityProjection:
    """Project public paired identity facts from the runtime credential store.

    The EasyNet-Cli SDK owns the secret-bearing persistence schema. Consumers
    receive only routable identity and display/endpoint facts and never decode
    or retain credential, deployment-signature, or trust-anchor material.
    When control discovery is available, its runtime identity must match the
    credential projection exactly before the paired principal is returned.
    """

    path = (
        Path(credentials_path)
        if credentials_path
        else runtime_credentials_path(control_path)
    )
    decoded = _read_json_object(path, "paired runtime credentials")
    realm = _required_projection_text(decoded, "realm")
    runtime_instance_id = _required_projection_text(decoded, "node_id")
    user_id = _optional_projection_text(decoded, "user_id")
    principal = user_ura(realm, user_id) if user_id else ""
    projection = RuntimeIdentityProjection(
        realm=realm,
        runtime_instance_id=runtime_instance_id,
        principal=principal,
        principal_display_name=_optional_projection_text(decoded, "username"),
        control_plane_endpoint=_required_projection_text(decoded, "hub_endpoint"),
    )
    if control_path:
        _validate_control_identity_binding(
            projection,
            read_runtime_control_discovery(control_path),
        )
    return projection


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
    unknown = sorted(set(decoded).difference(_RUNTIME_IDENTITY_PROJECTION_FIELDS))
    if unknown:
        raise _invalid(
            "runtime identity projection contains unknown fields: " + ", ".join(unknown)
        )
    realm = _required_projection_text(decoded, "realm")
    runtime_instance_id = _required_projection_text(decoded, "runtime_instance_id")
    return RuntimeIdentityProjection(
        realm=realm,
        runtime_instance_id=runtime_instance_id,
        principal=_optional_projection_text(decoded, "principal"),
        principal_display_name=_optional_projection_text(
            decoded, "principal_display_name"
        ),
        control_plane_endpoint=_optional_projection_text(
            decoded, "control_plane_endpoint"
        ),
    )


def _read_json_object(path: Path, label: str) -> Mapping[str, object]:
    try:
        raw = path.read_text(encoding="utf-8")
    except FileNotFoundError as exc:
        raise SDKError(
            code=ErrorCode.CALLER_IDENTITY_UNAVAILABLE,
            stage="runtime_environment",
            retry=RetryHint.NEVER,
            retryable=False,
            message=f"{label} not readable at {path}",
            details={"credentials_path": str(path)},
            cause=exc,
        ) from exc
    except OSError as exc:
        raise _invalid(f"read {label} at {path}: {exc}", exc) from exc
    try:
        decoded = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise _invalid(f"decode {label} JSON: {exc}", exc) from exc
    if not isinstance(decoded, Mapping):
        raise _invalid(f"{label} must be a JSON object")
    return decoded


def _validate_control_identity_binding(
    projection: RuntimeIdentityProjection,
    discovery: RuntimeControlDiscovery,
) -> None:
    identity = discovery.runtime_host_identity
    if identity is None:
        raise _identity_unavailable(
            "runtime control discovery has no runtime host identity"
        )
    if (
        identity.realm.strip() != projection.realm
        or identity.runtime_instance_id.strip() != projection.runtime_instance_id
    ):
        raise _identity_unavailable(
            "paired credentials do not match the attached runtime host identity"
        )


def _identity_unavailable(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.CALLER_IDENTITY_UNAVAILABLE,
        stage="runtime_environment",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )


def _required_projection_text(raw: Mapping[str, object], key: str) -> str:
    value = raw.get(key)
    if value is None:
        raise _invalid(f"runtime identity projection missing {key}")
    if not isinstance(value, str):
        raise _invalid(f"runtime identity projection {key} must be a string")
    text = value.strip()
    if not text:
        raise _invalid(f"runtime identity projection missing {key}")
    return text


def _optional_projection_text(raw: Mapping[str, object], key: str) -> str:
    value = raw.get(key)
    if value is None:
        return ""
    if not isinstance(value, str):
        raise _invalid(f"runtime identity projection {key} must be a string")
    return value.strip()


def _invalid(message: str, cause: Exception | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime_environment",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )
