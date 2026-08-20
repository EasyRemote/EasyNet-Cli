import json

import pytest

from easynet_sdk import HostStreamExec, RuntimeAbilityPackageManifest, SDKError


def test_runtime_ability_package_manifest_emits_canonical_deploy_shape() -> None:
    manifest = RuntimeAbilityPackageManifest(
        name="add",
        namespace="er",
        description="Add two numbers.",
        admission_action="stream",
        exposure="task",
        input_schema={
            "type": "object",
            "required": ["a", "b"],
            "properties": {
                "a": {"type": "number"},
                "b": {"type": "number"},
            },
        },
        output_schema={"type": "number"},
        exec=HostStreamExec(
            host_socket="/tmp/runtime-host.sock",
            function="er.add",
        ),
    )

    projected = manifest.to_mapping()

    assert projected == {
        "schema_version": "1",
        "name": "add",
        "namespace": "er",
        "description": "Add two numbers.",
        "admission_action": "stream",
        "exposure": "task",
        "input_schema": {
            "type": "object",
            "required": ["a", "b"],
            "properties": {
                "a": {"type": "number"},
                "b": {"type": "number"},
            },
        },
        "output_schema": {"type": "number"},
        "exec": {
            "kind": "host_stream",
            "host_socket": "/tmp/runtime-host.sock",
            "function": "er.add",
        },
    }
    assert "category" not in projected
    assert "tool_name" not in projected
    assert "version" not in projected
    assert json.loads(manifest.to_json()) == projected


def test_runtime_ability_package_manifest_rejects_unusable_identity_fields() -> None:
    manifest = RuntimeAbilityPackageManifest(
        name=" add ",
        namespace="er",
        description="Add two numbers.",
        admission_action="stream",
        input_schema={"type": "object"},
        exec=HostStreamExec(host_socket="/tmp/runtime-host.sock", function="er.add"),
    )

    with pytest.raises(SDKError, match="name must be a non-empty trimmed string"):
        manifest.to_mapping()


def test_runtime_ability_package_manifest_rejects_unknown_exposure() -> None:
    manifest = RuntimeAbilityPackageManifest(
        name="add",
        namespace="er",
        description="Add two numbers.",
        admission_action="stream",
        exposure="public",
        input_schema={"type": "object"},
        exec=HostStreamExec(host_socket="/tmp/runtime-host.sock", function="er.add"),
    )

    with pytest.raises(
        SDKError, match="exposure must be one of task, operator, or internal"
    ):
        manifest.to_mapping()
