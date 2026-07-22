"""Product-neutral Runtime Administration facade."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import StrEnum
from typing import Mapping

from .connection import ConnectOptions
from .runtime_lifecycle import (
    AttachOptions,
    Endpoints,
    RuntimeHandle,
    RuntimeHostDiscoverOptions,
    RuntimeHostStartRequest,
    RuntimeLifecycle,
    RuntimeLifecycleState,
    RuntimeStatus,
    StopOptions,
)
from .errors import ErrorCode, RetryHint, SDKError
from .health import DiagnosticsReport, HealthClient, RuntimeHealth
from .runtime import RuntimeClient
from ._runtime_admin_routes import (
    _PROFILE as _RUNTIME_ADMIN_PROFILE,
    _RUNTIME_ADMIN_SESSION_LIST_ABILITY,
)
from .runtime_ability import RuntimeAbilityClient, RuntimeCallContext


class RuntimeAdminCommand(StrEnum):
    """Typed runtime administration commands."""

    DISCOVER = "Discover"
    START = "Start"
    ATTACH = "Attach"
    STATUS = "Status"
    OPEN_RUNTIME = "OpenRuntime"
    STOP = "Stop"
    DETACH = "Detach"
    HEALTH = "Health"
    DIAGNOSTICS = "Diagnostics"


@dataclass(frozen=True)
class RuntimeReadiness:
    """Runtime lifecycle and health readiness projection."""

    lifecycle_state: RuntimeLifecycleState
    endpoints: Endpoints
    health: RuntimeHealth
    diagnostics: DiagnosticsReport | None
    ready: bool
    messages: tuple[str, ...] = ()


@dataclass(frozen=True)
class RuntimeSessionListRequest:
    """Runtime administration request for runtime session listing."""

    call: RuntimeCallContext
    include_terminated: bool | None = None


@dataclass(frozen=True)
class RuntimeSession:
    """One runtime session projection."""

    kind: str = ""
    session_id: str = ""
    device_ura: str = ""
    hub_ura: str = ""
    state: str = ""
    session_kind: str = ""
    created_unix_ms: int = 0
    expires_unix_ms: int = 0
    metadata: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class RuntimeSessionPage:
    """Runtime session page."""

    system_ability: str = ""
    state: str = ""
    sessions: tuple[RuntimeSession, ...] = ()
    next_cursor: object | None = None
    raw: Mapping[str, object] = field(default_factory=dict)


class RuntimeAdminClient:
    """Generic runtime administration facade."""

    def __init__(
        self, lifecycle: RuntimeLifecycle, health: HealthClient | None = None
    ) -> None:
        if lifecycle is None:
            raise _invalid_admin("runtime lifecycle is required")
        self._lifecycle = lifecycle
        self._health = health

    def discover(
        self, options: RuntimeHostDiscoverOptions = RuntimeHostDiscoverOptions()
    ) -> Endpoints:
        return self._lifecycle.discover(options)

    def start(self, config: RuntimeHostStartRequest) -> RuntimeHandle:
        return self._lifecycle.start(config)

    def attach(self, options: AttachOptions = AttachOptions()) -> RuntimeHandle:
        return self._lifecycle.attach(options)

    def status(self, handle: RuntimeHandle) -> RuntimeStatus:
        if handle is None:
            raise _invalid_admin("runtime handle is required")
        return handle.status()

    def open_runtime(
        self,
        handle: RuntimeHandle,
        options: ConnectOptions = ConnectOptions(),
    ) -> RuntimeClient:
        if handle is None:
            raise _invalid_admin("runtime handle is required")
        return handle.open_runtime(options)

    def stop(
        self, handle: RuntimeHandle, options: StopOptions = StopOptions()
    ) -> None:
        if handle is None:
            raise _invalid_admin("runtime handle is required")
        handle.stop(options)

    def detach(self, handle: RuntimeHandle) -> None:
        if handle is None:
            raise _invalid_admin("runtime handle is required")
        handle.detach()

    def health(self) -> RuntimeHealth:
        if self._health is None:
            raise _invalid_admin("health client is required")
        return self._health.runtime_health()

    def diagnostics(self) -> DiagnosticsReport:
        if self._health is None:
            raise _invalid_admin("health client is required")
        return self._health.diagnostics()

    def readiness(self, handle: RuntimeHandle) -> RuntimeReadiness:
        status = self.status(handle)
        health = self.health()
        diagnostics: DiagnosticsReport | None = None
        try:
            diagnostics = self.diagnostics()
        except SDKError:
            diagnostics = None
        messages = tuple(status.diagnostics) + tuple(health.diagnostics)
        return RuntimeReadiness(
            lifecycle_state=status.state,
            endpoints=status.endpoints,
            health=health,
            diagnostics=diagnostics,
            ready=_runtime_ready(status.state) and health.ready(),
            messages=messages,
        )


class RuntimeAdminAbilityClient:
    """Runtime administration abilities backed by RuntimeAbilityClient."""

    def __init__(self, ability: RuntimeAbilityClient) -> None:
        if ability is None:
            raise _invalid_admin("runtime ability client is required")
        self._ability = ability

    def list_sessions(
        self, request: RuntimeSessionListRequest
    ) -> RuntimeSessionPage:
        if not isinstance(request, RuntimeSessionListRequest):
            raise _invalid_admin("RuntimeSessionListRequest is required")
        args: dict[str, object] = {}
        if request.include_terminated is not None:
            args["include_terminated"] = request.include_terminated
        output = self._ability.invoke(
            _runtime_admin_call(request.call, _RUNTIME_ADMIN_SESSION_LIST_ABILITY),
            _RUNTIME_ADMIN_SESSION_LIST_ABILITY,
            args,
        )
        return _runtime_session_page(output)

def _runtime_ready(state: RuntimeLifecycleState) -> bool:
    return state in {
        RuntimeLifecycleState.INVOCATION_READY,
        RuntimeLifecycleState.RUNNING,
    }


def _runtime_admin_call(
    call: RuntimeCallContext, ability: str
) -> RuntimeCallContext:
    metadata = dict(call.metadata)
    metadata["sdk_profile"] = _RUNTIME_ADMIN_PROFILE
    metadata["system_ability"] = ability
    return RuntimeCallContext(
        caller_ura=call.caller_ura,
        callee_ura=call.callee_ura,
        subject_ura=call.subject_ura,
        nonce_base64=call.nonce_base64,
        causal_context=call.causal_context,
        descriptor_version=call.descriptor_version,
        metadata=metadata,
    )


def _runtime_session_page(output: Mapping[str, object]) -> RuntimeSessionPage:
    raw_rows = output.get("sessions")
    if not isinstance(raw_rows, list):
        raise _invalid_admin("runtime admin response field sessions must be an array")
    sessions: list[RuntimeSession] = []
    for row in raw_rows:
        if not isinstance(row, Mapping):
            raise _invalid_admin("runtime admin response field sessions entries must be objects")
        raw_metadata = row.get("metadata")
        sessions.append(
            RuntimeSession(
                kind=_admin_string(row.get("kind")),
                session_id=_admin_string(row.get("session_id")),
                device_ura=_admin_string(row.get("device_ura")),
                hub_ura=_admin_string(row.get("hub_ura")),
                state=_admin_string(row.get("state")),
                session_kind=_admin_string(row.get("session_kind")),
                created_unix_ms=_admin_int(row.get("created_unix_ms")),
                expires_unix_ms=_admin_int(row.get("expires_unix_ms")),
                metadata=dict(raw_metadata)
                if isinstance(raw_metadata, Mapping)
                else {},
            )
        )
    return RuntimeSessionPage(
        system_ability=_RUNTIME_ADMIN_SESSION_LIST_ABILITY,
        state=_admin_string(output.get("state")),
        sessions=tuple(sessions),
        next_cursor=output.get("next_cursor"),
        raw=dict(output),
    )


def _admin_string(value: object) -> str:
    return value.strip() if isinstance(value, str) else ""


def _required_admin_bool(output: Mapping[str, object], field: str) -> bool:
    value = output.get(field)
    if not isinstance(value, bool):
        raise _invalid_admin(f"runtime admin response field {field} must be a boolean")
    return value


def _optional_admin_bool(output: Mapping[str, object], field: str) -> bool:
    value = output.get(field)
    if value is None:
        return False
    if not isinstance(value, bool):
        raise _invalid_admin(f"runtime admin response field {field} must be a boolean")
    return value


def _admin_int(value: object) -> int:
    return value if isinstance(value, int) else 0


def _invalid_admin(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime_admin",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )
