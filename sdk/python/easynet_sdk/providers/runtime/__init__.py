"""Product-neutral runtime provider helpers."""

from .plugin_exec import (
    SidecarHandler,
    SidecarInvocation,
    SidecarProtocolError,
    serve_exec_plugin,
)

__all__ = [
    "SidecarHandler",
    "SidecarInvocation",
    "SidecarProtocolError",
    "serve_exec_plugin",
]
