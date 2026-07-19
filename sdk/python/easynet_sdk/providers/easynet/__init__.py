"""EasyNet provider binding for the canonical runtime SDK."""

from .keyring import (
    DaemonKeyringSignatureProvider,
    RuntimeSigningIdentity,
    ensure_runtime_signing_identity,
    load_runtime_signing_identity,
)
from .identity import read_daemon_runtime_identity_projection
from .lifecycle import (
    DaemonMode,
    DaemonStartProjection,
    DiscoverOptions,
    StartConfig,
)
from .plugin_exec import (
    SidecarHandler,
    SidecarInvocation,
    SidecarProtocolError,
    serve_exec_plugin,
)
from .transport import (
    connect_direct_invocation_transport,
    connect_invocation_transport,
)

__all__ = [
    "DaemonKeyringSignatureProvider",
    "DaemonMode",
    "DaemonStartProjection",
    "DiscoverOptions",
    "RuntimeSigningIdentity",
    "SidecarHandler",
    "SidecarInvocation",
    "SidecarProtocolError",
    "StartConfig",
    "connect_direct_invocation_transport",
    "connect_invocation_transport",
    "ensure_runtime_signing_identity",
    "load_runtime_signing_identity",
    "read_daemon_runtime_identity_projection",
    "serve_exec_plugin",
]
