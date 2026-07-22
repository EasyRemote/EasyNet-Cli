"""EasyNet provider lifecycle policy and source-compatible daemon DTOs."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import StrEnum
from typing import TYPE_CHECKING, Mapping

from ...connection import ConnectOptions
from ...errors import ErrorCode, RetryHint, SDKError

if TYPE_CHECKING:
    from ...runtime_lifecycle import (
        AttachOptions,
        Endpoints,
        RuntimeHandle,
        RuntimeLifecycleTransport,
    )
    from ...runtime import RuntimeClient


class DaemonMode(StrEnum):
    """easynet-daemon deployment role."""

    DEVICE = "device"
    HUB = "hub"
    BOTH = "both"


@dataclass(frozen=True)
class StartConfig:
    """easynet-daemon process start policy."""

    mode: DaemonMode
    realm: str = ""
    device_id: str = ""
    home_dir: str = ""
    daemon_bin: str = ""
    log_path: str = ""
    detached: bool = False
    env: Mapping[str, str] = field(default_factory=dict)
    uds_path: str = ""
    listen_tcp: str = ""
    tls_cert_path: str = ""
    tls_key_path: str = ""
    hub_endpoint: str = ""
    trust_path: str = ""

    def validate(self) -> None:
        if self.mode == DaemonMode.DEVICE and self.listen_tcp.strip():
            raise _invalid_lifecycle(
                "device mode must not accept a public TCP listener"
            )
        if (
            self.mode in {DaemonMode.HUB, DaemonMode.BOTH}
            and self.listen_tcp.strip()
            and (not self.tls_cert_path.strip() or not self.tls_key_path.strip())
        ):
            raise _invalid_lifecycle("public TCP listener requires TLS material")

    def to_json_dict(self) -> dict[str, object]:
        value: dict[str, object] = {"mode": self.mode.value}
        for key in (
            "realm",
            "device_id",
            "home_dir",
            "daemon_bin",
            "log_path",
            "uds_path",
            "listen_tcp",
            "tls_cert_path",
            "tls_key_path",
            "hub_endpoint",
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
class DiscoverOptions:
    """easynet-daemon discovery policy, including its product HOME root."""

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


def start_daemon(
    transport: "RuntimeLifecycleTransport", config: StartConfig
) -> "RuntimeHandle":
    from ...runtime_lifecycle import start_runtime_host

    return start_runtime_host(transport, config)


def attach_daemon(
    transport: "RuntimeLifecycleTransport",
    options: "AttachOptions | None" = None,
) -> "RuntimeHandle":
    from ...runtime_lifecycle import AttachOptions, attach_runtime_host

    return attach_runtime_host(transport, options or AttachOptions())


def discover_daemon(
    transport: "RuntimeLifecycleTransport",
    options: DiscoverOptions = DiscoverOptions(),
) -> "Endpoints":
    from ...runtime_lifecycle import discover_runtime_host

    return discover_runtime_host(transport, options)


def connect_local(
    transport: "RuntimeLifecycleTransport",
    options: ConnectOptions = ConnectOptions(),
) -> "RuntimeClient":
    from ...runtime_lifecycle import connect_runtime_local

    return connect_runtime_local(transport, options)


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
