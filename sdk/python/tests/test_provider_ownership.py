from __future__ import annotations

import ast
from pathlib import Path

import easynet_sdk
from easynet_sdk import runtime_lifecycle as canonical_lifecycle
from easynet_sdk import runtime_identity, transport
from easynet_sdk.providers.easynet import keyring as provider_keyring
from easynet_sdk.providers.easynet import lifecycle as provider_lifecycle


def test_daemon_compatibility_exports_are_exact_provider_or_runtime_aliases() -> None:
    assert easynet_sdk.StartConfig is provider_lifecycle.StartConfig
    assert easynet_sdk.DiscoverOptions is provider_lifecycle.DiscoverOptions
    assert easynet_sdk.DaemonMode is provider_lifecycle.DaemonMode
    assert easynet_sdk.RuntimeHostRole is provider_lifecycle.DaemonMode
    assert (
        easynet_sdk.RuntimeHostStartProjection
        is provider_lifecycle.DaemonStartProjection
    )
    assert easynet_sdk.DaemonControl is easynet_sdk.RuntimeLifecycle
    assert easynet_sdk.DaemonHandle is easynet_sdk.RuntimeHandle


def test_daemon_transport_aliases_share_one_canonical_state_machine() -> None:
    assert (
        easynet_sdk.DaemonInvocationTransport is easynet_sdk.RuntimeInvocationTransport
    )
    assert easynet_sdk.DaemonFrameStream is easynet_sdk.RuntimeFrameStream
    assert easynet_sdk.DaemonBidiChannel is easynet_sdk.RuntimeBidiChannel


def test_keyring_compatibility_module_has_no_second_implementation() -> None:
    assert (
        runtime_identity.RuntimeSigningIdentity
        is provider_keyring.RuntimeSigningIdentity
    )
    assert (
        runtime_identity.DaemonKeyringSignatureProvider
        is provider_keyring.DaemonKeyringSignatureProvider
    )
    assert (
        runtime_identity.load_runtime_signing_identity
        is provider_keyring.load_runtime_signing_identity
    )


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


def test_provider_connection_lowering_is_absent_from_canonical_transport() -> None:
    root = Path(transport.__file__).resolve()
    body = root.read_text(encoding="utf-8")
    assert "open_cabi_runtime_connector(" not in body
    assert "DirectRuntimeConnector(" not in body
