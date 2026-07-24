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

__all__ = [
    "RuntimeKeyringSignatureProvider",
    "RuntimeSigningIdentity",
    "SidecarHandler",
    "SidecarInvocation",
    "SidecarProtocolError",
    "ensure_runtime_signing_identity",
    "load_runtime_signing_identity",
    "serve_exec_plugin",
]
