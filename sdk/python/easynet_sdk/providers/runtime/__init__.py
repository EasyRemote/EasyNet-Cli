"""Product-neutral runtime provider helpers."""

from importlib import import_module
from typing import Any

__all__ = [
    "RuntimeKeyServiceSignatureProvider",
    "RuntimeHostDiscoverConfig",
    "RuntimeHostMode",
    "RuntimeHostStartConfig",
    "RuntimeSigningIdentity",
    "SidecarBidiHandler",
    "SidecarBidiInputFrames",
    "SidecarHandler",
    "SidecarInvocation",
    "SidecarProtocolError",
    "SidecarStreamHandler",
    "connect_direct_invocation_transport",
    "connect_invocation_transport",
    "ensure_runtime_signing_identity",
    "load_runtime_signing_identity",
    "serve_bidi_plugin",
    "serve_exec_plugin",
    "serve_plugin",
    "serve_stream_plugin",
]

_EXPORT_MODULES = {
    "RuntimeKeyServiceSignatureProvider": ".runtime_key_service",
    "RuntimeSigningIdentity": ".runtime_key_service",
    "ensure_runtime_signing_identity": ".runtime_key_service",
    "load_runtime_signing_identity": ".runtime_key_service",
    "RuntimeHostDiscoverConfig": ".lifecycle",
    "RuntimeHostMode": ".lifecycle",
    "RuntimeHostStartConfig": ".lifecycle",
    "SidecarBidiHandler": ".plugin_exec",
    "SidecarBidiInputFrames": ".plugin_exec",
    "SidecarHandler": ".plugin_exec",
    "SidecarInvocation": ".plugin_exec",
    "SidecarProtocolError": ".plugin_exec",
    "SidecarStreamHandler": ".plugin_exec",
    "serve_bidi_plugin": ".plugin_exec",
    "serve_exec_plugin": ".plugin_exec",
    "serve_plugin": ".plugin_exec",
    "serve_stream_plugin": ".plugin_exec",
    "connect_direct_invocation_transport": ".transport",
    "connect_invocation_transport": ".transport",
}


def __getattr__(name: str) -> Any:
    module_name = _EXPORT_MODULES.get(name)
    if module_name is None:
        raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
    value = getattr(import_module(module_name, __name__), name)
    globals()[name] = value
    return value
