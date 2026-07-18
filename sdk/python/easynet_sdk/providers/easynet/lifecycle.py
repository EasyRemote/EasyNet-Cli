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


@dataclass(frozen=True)
class DaemonStartProjection:
    """Host application projection into the EasyNet provider start policy."""

    mode: DaemonMode
    realm: str = ""
    device_id: str = ""
    env: Mapping[str, str] = field(default_factory=dict)
    log_path: str = ""
    detached: bool | None = None

    @classmethod
    def hub(
        cls,
        realm: str,
        *,
        env: Mapping[str, str] | None = None,
        log_path: str = "",
        detached: bool | None = None,
    ) -> "DaemonStartProjection":
        return cls.from_profile(
            mode="hub",
            realm=realm,
            env=env,
            log_path=log_path,
            detached=detached,
        )

    @classmethod
    def device(
        cls,
        device_id: str | None = None,
        *,
        env: Mapping[str, str] | None = None,
        log_path: str = "",
        detached: bool | None = None,
    ) -> "DaemonStartProjection":
        return cls.from_profile(
            mode="device",
            device_id=device_id or "",
            env=env,
            log_path=log_path,
            detached=detached,
        )

    @classmethod
    def from_profile(
        cls,
        *,
        mode: str,
        realm: str = "",
        device_id: str = "",
        env: Mapping[str, str] | None = None,
        log_path: str = "",
        detached: bool | None = None,
    ) -> "DaemonStartProjection":
        mode_value = _daemon_mode(mode)
        normalized_realm = realm.strip()
        normalized_device = device_id.strip()
        if mode_value == DaemonMode.HUB and not normalized_realm:
            raise _projection_invalid("hub realm must not be empty", "empty_realm")
        if mode_value == DaemonMode.DEVICE and not normalized_device:
            raise _projection_invalid(
                "device runtime host start requires a device_id",
                "missing_device_id",
            )
        return cls(
            mode=mode_value,
            realm=normalized_realm,
            device_id=normalized_device,
            env=dict(env or {}),
            log_path=log_path,
            detached=detached,
        )

    def validate(self) -> None:
        self.to_start_config().validate()

    def to_start_config(self) -> StartConfig:
        return StartConfig(
            mode=self.mode,
            realm=self.realm,
            device_id=self.device_id,
            log_path=self.log_path,
            detached=bool(self.detached),
            env=dict(self.env),
        )

    def to_wire_dict(self) -> dict[str, object]:
        value: dict[str, object] = {"mode": self.mode.value}
        if self.realm:
            value["realm"] = self.realm
        if self.device_id:
            value["device_id"] = self.device_id
        if self.env:
            value["env"] = dict(self.env)
        if self.log_path:
            value["log_path"] = self.log_path
        if self.detached is not None:
            value["detached"] = self.detached
        return value

    def to_json_bytes(self) -> bytes:
        return self.to_start_config().to_json_bytes()


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


def _daemon_mode(value: str) -> DaemonMode:
    try:
        mode = DaemonMode(value.strip())
    except ValueError as exc:
        raise _projection_invalid(
            f"unsupported runtime host role {value!r}",
            "invalid_runtime_host_role",
            exc,
        ) from exc
    if mode not in {DaemonMode.DEVICE, DaemonMode.HUB}:
        raise _projection_invalid(
            f"unsupported runtime host role {value!r}",
            "invalid_runtime_host_role",
        )
    return mode


def _projection_invalid(
    message: str,
    reason: str,
    cause: BaseException | None = None,
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime_lifecycle",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details={"reason": reason},
        cause=cause,
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
