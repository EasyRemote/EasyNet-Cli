import json

import pytest

import easynet_sdk
import easynet_sdk.environment as environment_module


def test_runtime_identity_projection_reads_credentials(tmp_path):
    credentials = tmp_path / "credentials.json"
    credentials.write_text(
        json.dumps(
            {
                "realm": "acme",
                "runtime_instance_id": "runtime-a",
                "principal": "alice",
                "control_plane_endpoint": "runtime:443",
            }
        )
    )

    projection = easynet_sdk.read_runtime_identity_projection(credentials)

    assert projection.realm == "acme"
    assert projection.runtime_instance_id == "runtime-a"
    assert projection.principal == "alice"
    assert projection.control_plane_endpoint == "runtime:443"


def test_sdk_environment_reads_runtime_identity_from_control_discovery(monkeypatch):
    monkeypatch.setattr(
        environment_module,
        "read_runtime_control_discovery",
        lambda control_path: easynet_sdk.RuntimeControlDiscovery(
            runtime_host_identity=easynet_sdk.RuntimeControlIdentityProjection(
                mode="device",
                realm="acme",
                runtime_instance_id="device-a",
            )
        ),
    )

    projection = easynet_sdk.SdkEnvironment(
        control_path="/tmp/control.json"
    ).runtime_identity_projection()

    assert projection.realm == "acme"
    assert projection.runtime_instance_id == "device-a"
    assert projection.principal == ""


def test_paired_runtime_identity_projection_exposes_only_public_facts(
    tmp_path,
):
    credentials = tmp_path / "credentials.json"
    credentials.write_text(
        json.dumps(
            {
                "realm": "acme",
                "node_id": "device-a",
                "user_id": "user-a",
                "username": "alice",
                "hub_endpoint": "https://hub.example",
                "credential_token": "must-not-escape",
                "deploy_signature": "must-not-escape",
            }
        )
    )
    control = _write_control_identity(tmp_path, runtime_instance_id="device-a")

    projection = easynet_sdk.SdkEnvironment(
        control_path=str(control)
    ).paired_runtime_identity_projection(credentials)

    assert projection == easynet_sdk.RuntimeIdentityProjection(
        realm="acme",
        runtime_instance_id="device-a",
        principal="easynet:///r/acme/user/user-a",
        principal_display_name="alice",
        control_plane_endpoint="https://hub.example",
    )
    assert "must-not-escape" not in repr(projection)


def test_paired_runtime_identity_projection_rejects_other_runtime(
    tmp_path,
):
    credentials = tmp_path / "credentials.json"
    credentials.write_text(
        json.dumps(
            {
                "realm": "acme",
                "node_id": "device-a",
                "user_id": "user-a",
                "username": "alice",
                "hub_endpoint": "https://hub.example",
            }
        )
    )
    control = _write_control_identity(tmp_path, runtime_instance_id="device-b")

    with pytest.raises(easynet_sdk.SDKError) as exc_info:
        easynet_sdk.SdkEnvironment(
            control_path=str(control)
        ).paired_runtime_identity_projection(credentials)

    assert exc_info.value.code == easynet_sdk.ErrorCode.CALLER_IDENTITY_UNAVAILABLE
    assert "do not match" in exc_info.value.message


def _write_control_identity(tmp_path, *, runtime_instance_id):
    control = tmp_path / "control.json"
    control.write_text(
        json.dumps(
            {
                "socket_path": "/tmp/control.sock",
                "invocation_endpoint": "/tmp/runtime.sock",
                "daemon_identity": {
                    "mode": "device",
                    "realm": "acme",
                    "node_id": runtime_instance_id,
                },
                "pid": 123,
                "daemon_version": "test",
                "supported_ipc_versions": {"min": 1, "max": 1},
                "capability_flags": ["paired_user_runtime_signer"],
            }
        )
    )
    return control


def test_runtime_identity_projection_rejects_daemon_node_id_alias():
    with pytest.raises(easynet_sdk.SDKError) as exc_info:
        easynet_sdk.runtime_identity_projection_from_json(
            '{"realm":"acme","node_id":"dev-a"}'
        )
    assert exc_info.value.code == easynet_sdk.ErrorCode.INVALID_ARGUMENT
    assert exc_info.value.stage == "runtime_environment"
    assert "unknown fields: node_id" in exc_info.value.message


def test_runtime_identity_projection_rejects_retired_aliases_with_runtime_id():
    with pytest.raises(easynet_sdk.SDKError) as exc_info:
        easynet_sdk.runtime_identity_projection_from_json(
            json.dumps(
                {
                    "realm": "acme",
                    "runtime_instance_id": "runtime-a",
                    "node_id": "dev-a",
                    "device_id": "device-a",
                }
            )
        )
    assert exc_info.value.code == easynet_sdk.ErrorCode.INVALID_ARGUMENT
    assert exc_info.value.stage == "runtime_environment"
    assert "device_id" in exc_info.value.message
    assert "node_id" in exc_info.value.message


def test_runtime_credentials_path_derives_from_control_path(tmp_path):
    assert easynet_sdk.runtime_credentials_path(tmp_path / "control.json") == (
        tmp_path / "credentials.json"
    )


def test_runtime_identity_projection_rejects_missing_runtime_instance_id():
    with pytest.raises(easynet_sdk.SDKError) as exc_info:
        easynet_sdk.runtime_identity_projection_from_json('{"realm":"acme"}')
    assert exc_info.value.code == easynet_sdk.ErrorCode.INVALID_ARGUMENT
    assert exc_info.value.stage == "runtime_environment"


@pytest.mark.parametrize(
    "raw",
    [
        {"realm": 7, "runtime_instance_id": "runtime-a"},
        {"realm": "acme", "runtime_instance_id": 7},
        {"realm": "acme", "runtime_instance_id": "runtime-a", "principal": True},
        {
            "realm": "acme",
            "runtime_instance_id": "runtime-a",
            "control_plane_endpoint": 443,
        },
    ],
)
def test_runtime_identity_projection_rejects_non_string_identity_facts(raw):
    with pytest.raises(easynet_sdk.SDKError) as exc_info:
        easynet_sdk.runtime_identity_projection_from_json(json.dumps(raw))
    assert exc_info.value.code == easynet_sdk.ErrorCode.INVALID_ARGUMENT
    assert exc_info.value.stage == "runtime_environment"
    assert "must be a string" in exc_info.value.message
