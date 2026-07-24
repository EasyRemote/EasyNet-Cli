"""Product-neutral runtime provider helpers."""

from importlib import import_module
from typing import Any

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

_EXPORT_MODULES = {
    "RuntimeKeyringSignatureProvider": ".keyring",
    "RuntimeSigningIdentity": ".keyring",
    "ensure_runtime_signing_identity": ".keyring",
    "load_runtime_signing_identity": ".keyring",
    "RuntimeHostDiscoverConfig": ".lifecycle",
    "RuntimeHostMode": ".lifecycle",
    "RuntimeHostStartConfig": ".lifecycle",
    "SidecarHandler": ".plugin_exec",
    "SidecarInvocation": ".plugin_exec",
    "SidecarProtocolError": ".plugin_exec",
    "serve_exec_plugin": ".plugin_exec",
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
