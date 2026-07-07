"""Desktop companion lifecycle facade.

Desktop companions are local daemon control-plane state. They are not Axon
Invocation primitives and this module intentionally exposes only the shared
SDK/control DTOs defined by the daemon companion contract.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Mapping

from .errors import ErrorCode, RetryHint, SDKError


class CompanionDesiredState(StrEnum):
    ENABLED = "enabled"
    DISABLED = "disabled"


class CompanionSupervisorState(StrEnum):
    UNSUPPORTED_PLATFORM = "unsupported_platform"
    UNSUPPORTED_SESSION = "unsupported_session"
    NOT_INSTALLED = "not_installed"
    INSTALLED_DISABLED = "installed_disabled"
    INSTALLED_ENABLED = "installed_enabled"
    INSTALL_ERROR = "install_error"
    ENABLE_ERROR = "enable_error"
    DISABLE_ERROR = "disable_error"


class CompanionObservedState(StrEnum):
    UNKNOWN = "unknown"
    NOT_RUNNING = "not_running"
    STARTING = "starting"
    RUNNING = "running"
    STALE = "stale"
    EXITED = "exited"
    VERSION_MISMATCH = "version_mismatch"
    HEALTH_ERROR = "health_error"


class CompanionProjectedState(StrEnum):
    DISABLED = "disabled"
    UNSUPPORTED_PLATFORM = "unsupported_platform"
    UNSUPPORTED_SESSION = "unsupported_session"
    NOT_INSTALLED = "not_installed"
    INSTALLED_DISABLED = "installed_disabled"
    READY_STOPPED = "ready_stopped"
    STARTING = "starting"
    RUNNING = "running"
    STALE = "stale"
    ERROR = "error"


class CompanionBootPolicy(StrEnum):
    MANUAL = "manual"
    ENSURE_RUNNING_AFTER_DAEMON_READY = "ensure_running_after_daemon_ready"


class CompanionStopPolicy(StrEnum):
    KEEP_RUNNING = "keep_running"
    STOP_ON_RUNTIME_STOP = "stop_on_runtime_stop"
    STOP_ON_PLUGIN_DISABLE = "stop_on_plugin_disable"


class CompanionHealthMode(StrEnum):
    PROCESS_NAME = "process_name"
    STATUS_FILE = "status_file"
    LOCAL_IPC = "local_ipc"


@dataclass(frozen=True)
class DesktopCompanionStatus:
    package_id: str
    package_version: str
    display_name: str
    platform: str
    desired_state: CompanionDesiredState
    supervisor_state: CompanionSupervisorState
    observed_state: CompanionObservedState
    projected_state: CompanionProjectedState
    boot_policy: CompanionBootPolicy
    stop_policy: CompanionStopPolicy
    health: CompanionHealthMode
    pid: int | None = None
    version: str | None = None
    last_seen_unix_ms: int | None = None
    launch_method: str | None = None
    error: Mapping[str, object] | None = None
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(cls, raw: bytes | str | Mapping[str, object]) -> "DesktopCompanionStatus":
        decoded = _json_object(raw, "desktop companion status")
        return cls(
            package_id=_required_string(decoded, "package_id"),
            package_version=_required_string(decoded, "package_version"),
            display_name=_required_string(decoded, "display_name"),
            platform=_required_string(decoded, "platform"),
            desired_state=_enum_field(
                decoded, "desired_state", CompanionDesiredState
            ),
            supervisor_state=_enum_field(
                decoded, "supervisor_state", CompanionSupervisorState
            ),
            observed_state=_enum_field(
                decoded, "observed_state", CompanionObservedState
            ),
            projected_state=_enum_field(
                decoded, "projected_state", CompanionProjectedState
            ),
            boot_policy=_enum_field(decoded, "boot_policy", CompanionBootPolicy),
            stop_policy=_enum_field(decoded, "stop_policy", CompanionStopPolicy),
            health=_enum_field(decoded, "health", CompanionHealthMode),
            pid=_optional_non_negative_int(decoded.get("pid"), "pid"),
            version=_optional_string(decoded.get("version"), "version"),
            last_seen_unix_ms=_optional_non_negative_int(
                decoded.get("last_seen_unix_ms"), "last_seen_unix_ms"
            ),
            launch_method=_optional_string(decoded.get("launch_method"), "launch_method"),
            error=_optional_mapping(decoded.get("error"), "error"),
            metadata=_optional_mapping(decoded.get("metadata"), "metadata") or {},
        )


@dataclass(frozen=True)
class DesktopCompanionList:
    companions: tuple[DesktopCompanionStatus, ...]

    @classmethod
    def from_json(cls, raw: bytes | str | Mapping[str, object]) -> "DesktopCompanionList":
        decoded = _json_object(raw, "desktop companion list")
        companions = decoded.get("companions")
        if not isinstance(companions, list):
            raise _invalid_companion("companions must be an array")
        return cls(
            companions=tuple(
                DesktopCompanionStatus.from_json(item) for item in companions
            )
        )


@dataclass(frozen=True)
class DesktopCompanionActionResult:
    package_id: str
    action: str
    changed: bool
    status_before: DesktopCompanionStatus | None = None
    status_after: DesktopCompanionStatus | None = None
    error: Mapping[str, object] | None = None
    metadata: Mapping[str, object] = field(default_factory=dict)

    @classmethod
    def from_json(
        cls, raw: bytes | str | Mapping[str, object]
    ) -> "DesktopCompanionActionResult":
        decoded = _json_object(raw, "desktop companion action result")
        changed = decoded.get("changed")
        if not isinstance(changed, bool):
            raise _invalid_companion("changed must be a boolean")
        return cls(
            package_id=_required_string(decoded, "package_id"),
            action=_required_string(decoded, "action"),
            changed=changed,
            status_before=_optional_status(decoded.get("status_before"), "status_before"),
            status_after=_optional_status(decoded.get("status_after"), "status_after"),
            error=_optional_mapping(decoded.get("error"), "error"),
            metadata=_optional_mapping(decoded.get("metadata"), "metadata") or {},
        )


def _optional_status(
    value: object, field_name: str
) -> DesktopCompanionStatus | None:
    if value is None:
        return None
    if not isinstance(value, Mapping):
        raise _invalid_companion(f"{field_name} must be an object or null")
    return DesktopCompanionStatus.from_json(value)


def _json_object(raw: bytes | str | Mapping[str, object], label: str) -> dict[str, object]:
    if isinstance(raw, Mapping):
        return dict(raw)
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_companion(f"decode {label} JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_companion(f"{label} JSON must be an object")
    return decoded


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_companion(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_companion(f"{field_name} must be a string or null")
    return value


def _optional_non_negative_int(value: object, field_name: str) -> int | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise _invalid_companion(f"{field_name} must be a non-negative integer or null")
    return value


def _optional_mapping(value: object, field_name: str) -> Mapping[str, object] | None:
    if value is None:
        return None
    if not isinstance(value, Mapping):
        raise _invalid_companion(f"{field_name} must be an object or null")
    return dict(value)


def _enum_field(
    decoded: Mapping[str, object],
    field_name: str,
    enum_type: type[StrEnum],
) -> StrEnum:
    raw = _required_string(decoded, field_name)
    try:
        return enum_type(raw)
    except ValueError as exc:
        raise _invalid_companion(f"{field_name} has invalid value {raw!r}", exc) from exc


def _invalid_companion(
    message: str, cause: BaseException | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="desktop_companion",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )
