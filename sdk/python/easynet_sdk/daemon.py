"""REQ-LANG-5 compatibility exports for the EasyNet lifecycle provider."""

from .providers.easynet.lifecycle import (
    DaemonMode,
    DaemonStartProjection,
    DiscoverOptions,
    StartConfig,
    attach_daemon,
    connect_local,
    discover_daemon,
    start_daemon,
)
from .runtime_lifecycle import (
    AttachOptions,
    Endpoints,
    RuntimeHandle,
    RuntimeHost,
    RuntimeHostDiscoverOptions,
    RuntimeHostDiscoverRequest,
    RuntimeHostHandle,
    RuntimeHostStartRequest,
    RuntimeLifecycle,
    RuntimeLifecycleState,
    RuntimeLifecycleTransport,
    RuntimeStatus,
    StopOptions,
    attach_runtime_host,
    connect_runtime_local,
    discover_runtime_host,
    start_runtime_host,
)

RuntimeHostRole = DaemonMode
RuntimeHostStartProjection = DaemonStartProjection
DaemonLifecycleState = RuntimeLifecycleState
DaemonStatus = RuntimeStatus
DaemonTransport = RuntimeLifecycleTransport
DaemonControl = RuntimeLifecycle
DaemonLifecycleFacade = RuntimeHost
DaemonHandleFacade = RuntimeHostHandle
DaemonHandle = RuntimeHandle

__all__ = [
    "AttachOptions",
    "DaemonControl",
    "DaemonHandle",
    "DaemonHandleFacade",
    "DaemonLifecycleFacade",
    "DaemonLifecycleState",
    "DaemonMode",
    "DaemonStartProjection",
    "DaemonStatus",
    "DaemonTransport",
    "DiscoverOptions",
    "Endpoints",
    "RuntimeHandle",
    "RuntimeHost",
    "RuntimeHostDiscoverOptions",
    "RuntimeHostDiscoverRequest",
    "RuntimeHostHandle",
    "RuntimeHostRole",
    "RuntimeHostStartProjection",
    "RuntimeHostStartRequest",
    "RuntimeLifecycle",
    "RuntimeLifecycleState",
    "RuntimeLifecycleTransport",
    "RuntimeStatus",
    "StartConfig",
    "StopOptions",
    "attach_daemon",
    "attach_runtime_host",
    "connect_local",
    "connect_runtime_local",
    "discover_daemon",
    "discover_runtime_host",
    "start_daemon",
    "start_runtime_host",
]
