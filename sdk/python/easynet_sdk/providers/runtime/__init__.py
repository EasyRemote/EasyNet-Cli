"""Product-neutral runtime provider helpers."""

from .plugin_exec import (
    SidecarHandler,
    SidecarInvocation,
    SidecarProtocolError,
    serve_exec_plugin,
)
from .keyring import (
    RuntimeSigningIdentity,
    RuntimeKeyringSignatureProvider,
    ensure_runtime_signing_identity,
    load_runtime_signing_identity,
)
from .lifecycle import (
    RuntimeHostDiscoverConfig,
    RuntimeHostMode,
    RuntimeHostStartConfig,
)
from .transport import (
    connect_direct_invocation_transport,
    connect_invocation_transport,
)

__all__ = [
    "RuntimeKeyringSignatureProvider",
    "RuntimeHostDiscoverConfig",
    "RuntimeHostMode",
    "RuntimeHostStartConfig",
    "RuntimeSigningIdentity",
    "SidecarHandler",
    "SidecarInvocation",
    "SidecarProtocolError",
    "connect_direct_invocation_transport",
    "connect_invocation_transport",
    "ensure_runtime_signing_identity",
    "load_runtime_signing_identity",
    "serve_exec_plugin",
]
