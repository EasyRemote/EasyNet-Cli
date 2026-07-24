"""Runtime provider lifecycle policy and stable host DTOs."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Mapping

from ...errors import ErrorCode, RetryHint, SDKError


class RuntimeHostMode(StrEnum):
    """Runtime host deployment role understood by the local host provider."""

    EDGE = "device"
    AUTHORITY = "hub"
    COMBINED = "both"


@dataclass(frozen=True)
class RuntimeHostStartConfig:
    """Runtime host process start policy."""

    mode: RuntimeHostMode
    realm: str = ""
    runtime_instance_id: str = ""
    home_dir: str = ""
    runtime_bin: str = ""
    log_path: str = ""
    detached: bool = False
    env: Mapping[str, str] = field(default_factory=dict)
    uds_path: str = ""
    listen_tcp: str = ""
    tls_cert_path: str = ""
    tls_key_path: str = ""
    authority_endpoint: str = ""
    trust_path: str = ""

    def validate(self) -> None:
        if self.mode == RuntimeHostMode.EDGE and self.listen_tcp.strip():
            raise _invalid_lifecycle(
                "edge runtime host mode must not accept a public TCP listener"
            )
        if (
            self.mode in {RuntimeHostMode.AUTHORITY, RuntimeHostMode.COMBINED}
            and self.listen_tcp.strip()
            and (not self.tls_cert_path.strip() or not self.tls_key_path.strip())
        ):
            raise _invalid_lifecycle("public TCP listener requires TLS material")

    def to_json_dict(self) -> dict[str, object]:
        value: dict[str, object] = {"mode": self.mode.value}
        for key in (
            "realm",
            "runtime_instance_id",
            "home_dir",
            "runtime_bin",
            "log_path",
            "uds_path",
            "listen_tcp",
            "tls_cert_path",
            "tls_key_path",
            "authority_endpoint",
            "trust_path",
        ):
            item = getattr(self, key)
            if item:
                value[key] = item
        if self.detached:
            value["detached"] = True
        if self.env:
            value["env"] = dict(self.env)
        return value

    def to_json_bytes(self) -> bytes:
        return json.dumps(
            self.to_json_dict(), separators=(",", ":"), sort_keys=True
        ).encode("utf-8")


@dataclass(frozen=True)
class RuntimeHostDiscoverConfig:
    """Runtime host provider discovery policy, including the host state root."""

    control_endpoint: str = ""
    control_path: str = ""
    home_dir: str = ""

    def to_json_bytes(self) -> bytes:
        return _json_bytes(
            {
                "control_endpoint": self.control_endpoint,
                "control_path": self.control_path,
                "home_dir": self.home_dir,
            }
        )


def _invalid_lifecycle(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="sdk",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )


def _json_bytes(value: Mapping[str, object]) -> bytes:
    compact = {key: item for key, item in value.items() if item not in ("", 0, False)}
    return json.dumps(compact, separators=(",", ":"), sort_keys=True).encode("utf-8")
