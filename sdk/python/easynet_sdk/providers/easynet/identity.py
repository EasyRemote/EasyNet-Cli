"""EasyNet daemon identity projection adapter."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Mapping

from ...errors import ErrorCode, SDKError
from ...runtime_environment import (
    RuntimeIdentityProjection,
    read_runtime_identity_projection,
)


def read_daemon_runtime_identity_projection(
    credentials_path: str | Path,
) -> RuntimeIdentityProjection:
    """Read EasyNet daemon credentials as a canonical runtime projection."""

    path = Path(credentials_path)
    try:
        return read_runtime_identity_projection(path)
    except SDKError as exc:
        if exc.code != ErrorCode.INVALID_ARGUMENT:
            raise
        raw = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(raw, Mapping):
        raise ValueError("daemon credentials projection must be an object")
    realm = _text(raw, "realm")
    runtime_instance_id = _runtime_instance_id(raw)
    if not realm or not runtime_instance_id:
        raise ValueError("daemon credentials missing runtime identity")
    return RuntimeIdentityProjection(
        realm=realm,
        runtime_instance_id=runtime_instance_id,
        principal=_text(raw, "username"),
        control_plane_endpoint=_text(raw, "hub_endpoint"),
    )


def _runtime_instance_id(raw: Mapping[str, object]) -> str:
    device_id = _text(raw, "device_id")
    node_id = _text(raw, "node_id")
    if device_id and node_id and device_id != node_id:
        raise ValueError("daemon credentials contain conflicting device_id and node_id")
    return device_id or node_id


def _text(raw: Mapping[str, object], key: str) -> str:
    value = raw.get(key)
    if value is None:
        return ""
    return str(value).strip()
