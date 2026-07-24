from __future__ import annotations

import ast
from pathlib import Path

import easynet_sdk
from easynet_sdk import runtime_lifecycle as canonical_lifecycle
from easynet_sdk import transport
from easynet_sdk.providers.runtime import lifecycle as provider_lifecycle
from easynet_sdk.providers.runtime import keyring as provider_keyring
from easynet_sdk.providers.runtime import plugin_exec as provider_plugin_exec


def test_runtime_lifecycle_provider_exports_are_provider_scoped() -> None:
    assert provider_lifecycle.RuntimeHostStartConfig.__module__.endswith(
        "providers.runtime.lifecycle"
    )
    assert provider_lifecycle.RuntimeHostDiscoverConfig.__module__.endswith(
        "providers.runtime.lifecycle"
    )
    assert provider_lifecycle.RuntimeHostMode.__module__.endswith(
        "providers.runtime.lifecycle"
    )
    for name in (
        "StartConfig",
        "DiscoverOptions",
        "DaemonMode",
        "RuntimeHostStartConfig",
        "RuntimeHostDiscoverConfig",
        "RuntimeHostMode",
        "start_daemon",
        "discover_daemon",
        "attach_daemon",
    ):
        assert not hasattr(easynet_sdk, name), name
    assert not hasattr(provider_lifecycle, "DaemonStartProjection")
    for name in ("start_daemon", "discover_daemon", "attach_daemon", "connect_local"):
        assert not hasattr(provider_lifecycle, name), name


def test_transport_root_exports_only_runtime_transport_names() -> None:
    assert hasattr(easynet_sdk, "RuntimeInvocationTransport")
    assert hasattr(transport, "RuntimeInvocationTransport")
    for name in ("DaemonInvocationTransport", "DaemonFrameStream", "DaemonBidiChannel"):
        assert not hasattr(easynet_sdk, name), name
        assert not hasattr(transport, name), name


def test_keyring_provider_is_not_reexported_as_canonical_root() -> None:
    assert provider_keyring.RuntimeSigningIdentity.__module__.endswith(
        "providers.runtime.keyring"
    )
    assert provider_keyring.RuntimeKeyringSignatureProvider.__module__.endswith(
        "providers.runtime.keyring"
    )
    assert not hasattr(easynet_sdk, "DaemonKeyringSignatureProvider")
    assert not hasattr(easynet_sdk, "RuntimeKeyringSignatureProvider")
    assert not hasattr(easynet_sdk, "RuntimeSigningIdentity")


def test_plugin_exec_provider_is_not_reexported_as_canonical_root() -> None:
    assert provider_plugin_exec.SidecarInvocation.__module__.endswith(
        "providers.runtime.plugin_exec"
    )
    assert not hasattr(easynet_sdk, "PluginInvocation")
    assert not hasattr(easynet_sdk, "SidecarInvocation")
    assert not hasattr(easynet_sdk, "serve_exec_plugin")


def test_product_lifecycle_dtos_are_not_defined_in_canonical_module() -> None:
    root = Path(canonical_lifecycle.__file__).resolve()
    parsed = ast.parse(root.read_text(encoding="utf-8"), filename=str(root))
    defined_classes = {
        node.name for node in parsed.body if isinstance(node, ast.ClassDef)
    }
    assert "StartConfig" not in defined_classes
    assert "DiscoverOptions" not in defined_classes
    assert "RuntimeHostRole" not in defined_classes
    assert "RuntimeHostStartProjection" not in defined_classes


def test_canonical_lifecycle_does_not_import_product_provider() -> None:
    root = Path(canonical_lifecycle.__file__).resolve()
    body = root.read_text(encoding="utf-8")
    assert ".providers" not in body
    assert "DaemonMode" not in body
    assert "StartConfig" not in body


def test_python_product_provider_package_is_retired() -> None:
    provider_root = Path(easynet_sdk.__file__).resolve().parent / "providers" / "easynet"
    assert not provider_root.exists()


def test_provider_connection_lowering_is_absent_from_canonical_transport() -> None:
    root = Path(transport.__file__).resolve()
    body = root.read_text(encoding="utf-8")
    assert "open_cabi_runtime_connector(" not in body
    assert "DirectRuntimeConnector(" not in body
