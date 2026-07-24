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
    assert "missing runtime_instance_id" in exc_info.value.message


def test_runtime_credentials_path_derives_from_control_path(tmp_path):
    assert easynet_sdk.runtime_credentials_path(tmp_path / "control.json") == (
        tmp_path / "credentials.json"
    )


def test_runtime_identity_projection_rejects_missing_runtime_instance_id():
    with pytest.raises(easynet_sdk.SDKError) as exc_info:
        easynet_sdk.runtime_identity_projection_from_json('{"realm":"acme"}')
    assert exc_info.value.code == easynet_sdk.ErrorCode.INVALID_ARGUMENT
    assert exc_info.value.stage == "runtime_environment"
