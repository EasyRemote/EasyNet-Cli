import json

import pytest

import easynet_sdk


def test_runtime_identity_projection_reads_credentials(tmp_path):
    credentials = tmp_path / "credentials.json"
    credentials.write_text(
        json.dumps(
            {
                "realm": "acme",
                "device_id": "dev-a",
                "username": "alice",
                "hub_endpoint": "hub:443",
            }
        )
    )

    projection = easynet_sdk.read_runtime_identity_projection(credentials)

    assert projection.realm == "acme"
    assert projection.device_id == "dev-a"
    assert projection.username == "alice"
    assert projection.hub_endpoint == "hub:443"


def test_runtime_identity_projection_rejects_node_id_alias():
    with pytest.raises(easynet_sdk.SDKError) as exc_info:
        easynet_sdk.runtime_identity_projection_from_json(
            '{"realm":"acme","node_id":"dev-a"}'
        )
    assert exc_info.value.code == easynet_sdk.ErrorCode.INVALID_ARGUMENT
    assert exc_info.value.stage == "runtime_environment"


def test_runtime_credentials_path_derives_from_control_path(tmp_path):
    assert easynet_sdk.runtime_credentials_path(tmp_path / "control.json") == (
        tmp_path / "credentials.json"
    )


def test_runtime_identity_projection_rejects_missing_device_id():
    with pytest.raises(easynet_sdk.SDKError) as exc_info:
        easynet_sdk.runtime_identity_projection_from_json('{"realm":"acme"}')
    assert exc_info.value.code == easynet_sdk.ErrorCode.INVALID_ARGUMENT
    assert exc_info.value.stage == "runtime_environment"
