import json

import pytest

import easynet_sdk


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
