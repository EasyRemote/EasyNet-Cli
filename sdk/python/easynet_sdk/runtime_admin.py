"""Product-neutral Runtime Administration facade."""

from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum

from .connection import ConnectOptions
from .daemon import (
    AttachOptions,
    DaemonControl,
    DaemonHandle,
    DaemonLifecycleState,
    DaemonStatus,
    DiscoverOptions,
    Endpoints,
    StartConfig,
    StopOptions,
)
from .errors import ErrorCode, RetryHint, SDKError
from .health import DiagnosticsReport, HealthClient, RuntimeHealth
from .runtime import RuntimeClient


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

    lifecycle_state: DaemonLifecycleState
    endpoints: Endpoints
    health: RuntimeHealth
    diagnostics: DiagnosticsReport | None
    ready: bool
    messages: tuple[str, ...] = ()


class RuntimeAdminClient:
    """Generic daemon/runtime administration facade."""

    def __init__(
        self, control: DaemonControl, health: HealthClient | None = None
    ) -> None:
        if control is None:
            raise _invalid_admin("daemon control is required")
        self._control = control
        self._health = health

    def discover(self, options: DiscoverOptions = DiscoverOptions()) -> Endpoints:
        return self._control.discover(options)

    def start(self, config: StartConfig) -> DaemonHandle:
        return self._control.start(config)

    def attach(self, options: AttachOptions = AttachOptions()) -> DaemonHandle:
        return self._control.attach(options)

    def status(self, handle: DaemonHandle) -> DaemonStatus:
        if handle is None:
            raise _invalid_admin("daemon handle is required")
        return handle.status()

    def open_runtime(
        self,
        handle: DaemonHandle,
        options: ConnectOptions = ConnectOptions(),
    ) -> RuntimeClient:
        if handle is None:
            raise _invalid_admin("daemon handle is required")
        return handle.open_runtime(options)

    def stop(
        self, handle: DaemonHandle, options: StopOptions = StopOptions()
    ) -> None:
        if handle is None:
            raise _invalid_admin("daemon handle is required")
        handle.stop(options)

    def detach(self, handle: DaemonHandle) -> None:
        if handle is None:
            raise _invalid_admin("daemon handle is required")
        handle.detach()

    def health(self) -> RuntimeHealth:
        if self._health is None:
            raise _invalid_admin("health client is required")
        return self._health.runtime_health()

    def diagnostics(self) -> DiagnosticsReport:
        if self._health is None:
            raise _invalid_admin("health client is required")
        return self._health.diagnostics()

    def readiness(self, handle: DaemonHandle) -> RuntimeReadiness:
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


def _runtime_ready(state: DaemonLifecycleState) -> bool:
    return state in {
        DaemonLifecycleState.INVOCATION_READY,
        DaemonLifecycleState.RUNNING,
    }


def _invalid_admin(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime_admin",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )
