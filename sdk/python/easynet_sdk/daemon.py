"""Daemon lifecycle state facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Mapping, Protocol, runtime_checkable

from .connection import ConnectOptions
from .errors import ErrorCode, RetryHint, SDKError
from .runtime import RuntimeClient, RuntimeTransport


class DaemonMode(StrEnum):
    """Local daemon deployment role."""

    DEVICE = "device"
    HUB = "hub"
    BOTH = "both"


class DaemonLifecycleState(StrEnum):
    """SDK daemon lifecycle state projection."""

    UNKNOWN = "Unknown"
    DISCOVERED = "Discovered"
    STARTING = "Starting"
    CONTROL_READY = "ControlReady"
    INVOCATION_READY = "InvocationReady"
    RUNNING = "Running"
    STOPPING = "Stopping"
    STOPPED = "Stopped"
    CONFIG_INVALID = "ConfigInvalid"
    PERMISSION_DENIED = "PermissionDenied"
    VERSION_MISMATCH = "VersionMismatch"
    CONTROL_ONLY = "ControlOnly"
    INVOCATION_DOWN = "InvocationDown"
    START_FAILED = "StartFailed"
    CRASH_LOOP = "CrashLoop"


@dataclass(frozen=True)
class StartConfig:
    """Daemon lifecycle start policy."""

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
class AttachOptions:
    """Existing daemon attachment request."""

    control_endpoint: str = ""
    invocation_endpoint: str = ""
    control_path: str = ""

    def to_json_bytes(self) -> bytes:
        return _json_bytes(
            {
                "control_endpoint": self.control_endpoint,
                "invocation_endpoint": self.invocation_endpoint,
                "control_path": self.control_path,
            }
        )


@dataclass(frozen=True)
class DiscoverOptions:
    """Daemon endpoint discovery knobs."""

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
class StopOptions:
    """Daemon stop policy."""

    graceful_timeout_ms: int = 0
    force: bool = False

    def to_json_bytes(self) -> bytes:
        return _json_bytes(
            {
                "graceful_timeout_ms": self.graceful_timeout_ms,
                "force": self.force,
            }
        )


@dataclass(frozen=True)
class Endpoints:
    """Daemon control and Invocation transport locators."""

    control_endpoint: str = ""
    invocation_endpoint: str = ""
    public_endpoint: str = ""

    @classmethod
    def from_json(cls, raw: bytes | str, *, require_invocation: bool = True) -> "Endpoints":
        decoded = _json_object(raw, "daemon endpoints")
        endpoint = cls(
            control_endpoint=_optional_string(
                decoded.get("control_endpoint"), "control_endpoint"
            )
            or "",
            invocation_endpoint=_optional_string(
                decoded.get("invocation_endpoint"), "invocation_endpoint"
            )
            or "",
            public_endpoint=_optional_string(
                decoded.get("public_endpoint"), "public_endpoint"
            )
            or "",
        )
        if require_invocation and not endpoint.invocation_endpoint:
            raise _invalid_daemon("invocation_endpoint is required")
        return endpoint


@dataclass(frozen=True)
class DaemonStatus:
    """Typed daemon lifecycle status projection."""

    state: DaemonLifecycleState
    handle_id: str = ""
    mode: DaemonMode | None = None
    pid: int = 0
    version: str = ""
    message: str = ""
    endpoints: Endpoints = field(default_factory=Endpoints)
    diagnostics: tuple[str, ...] = field(default_factory=tuple)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "DaemonStatus":
        decoded = _json_object(raw, "daemon status")
        raw_state = _required_string(decoded, "state")
        try:
            state = DaemonLifecycleState(raw_state)
        except ValueError as exc:
            raise _invalid_daemon("invalid daemon lifecycle state", exc) from exc
        mode_value = _optional_string(decoded.get("mode"), "mode")
        mode = DaemonMode(mode_value) if mode_value else None
        endpoints_value = decoded.get("endpoints", {})
        if endpoints_value is None:
            endpoints_value = {}
        if not isinstance(endpoints_value, dict):
            raise _invalid_daemon("endpoints must be an object")
        diagnostics_value = decoded.get("diagnostics", [])
        if not isinstance(diagnostics_value, list) or not all(
            isinstance(item, str) for item in diagnostics_value
        ):
            raise _invalid_daemon("diagnostics must be an array of strings")
        return cls(
            state=state,
            handle_id=_optional_string(decoded.get("handle_id"), "handle_id") or "",
            mode=mode,
            pid=_optional_non_negative_int(decoded.get("pid"), "pid"),
            version=_optional_string(decoded.get("version"), "version") or "",
            message=_optional_string(decoded.get("message"), "message") or "",
            endpoints=Endpoints.from_json(
                json.dumps(endpoints_value), require_invocation=False
            ),
            diagnostics=tuple(diagnostics_value),
        )


@runtime_checkable
class DaemonTransport(Protocol):
    """Concrete daemon lifecycle operations supplied by the integration layer."""

    def discover(self, options_json: bytes) -> bytes:
        ...

    def start(self, config_json: bytes) -> bytes:
        ...

    def attach(self, options_json: bytes) -> bytes:
        ...

    def status(self, handle_id: str) -> bytes:
        ...

    def open_runtime(
        self, handle_id: str, options_json: bytes
    ) -> tuple[RuntimeTransport, bytes]:
        ...

    def stop(self, handle_id: str, options_json: bytes) -> bytes:
        ...

    def detach(self, handle_id: str) -> None:
        ...


@dataclass(frozen=True)
class DaemonControl:
    """Daemon lifecycle facade root over an integration transport."""

    transport: DaemonTransport

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_daemon("daemon transport is required")

    def discover(self, options: DiscoverOptions = DiscoverOptions()) -> Endpoints:
        try:
            raw = self.transport.discover(options.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("daemon discover failed", exc) from exc
        return Endpoints.from_json(raw)

    def start(self, config: StartConfig) -> "DaemonHandle":
        _validate_start_config(config)
        try:
            raw = self.transport.start(config.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("daemon start failed", exc) from exc
        status = DaemonStatus.from_json(raw)
        _require_runtime_ready(status)
        return DaemonHandle(self.transport, status)

    def attach(self, options: AttachOptions = AttachOptions()) -> "DaemonHandle":
        try:
            raw = self.transport.attach(options.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("daemon attach failed", exc) from exc
        status = DaemonStatus.from_json(raw)
        _require_runtime_ready(status)
        return DaemonHandle(self.transport, status)

    def connect_local(self, options: ConnectOptions = ConnectOptions()) -> RuntimeClient:
        endpoints = self.discover(DiscoverOptions(control_path=options.control_path))
        runtime_endpoint = options.endpoint or endpoints.invocation_endpoint
        if not runtime_endpoint:
            raise _invalid_daemon("invocation_endpoint is required")
        handle = self.attach(
            AttachOptions(
                control_endpoint=endpoints.control_endpoint,
                invocation_endpoint=runtime_endpoint,
                control_path=options.control_path,
            )
        )
        open_options = ConnectOptions(
            endpoint=runtime_endpoint,
            control_path=options.control_path,
            dial_timeout_ms=options.dial_timeout_ms,
            invoke_timeout_ms=options.invoke_timeout_ms,
            max_message_bytes=options.max_message_bytes,
            reconnect=options.reconnect,
        )
        try:
            client = handle.open_runtime(open_options)
        except Exception:
            handle.detach()
            raise
        handle.detach()
        return client


@dataclass
class DaemonHandle:
    """Local daemon lifecycle handle state."""

    transport: DaemonTransport
    _status: DaemonStatus
    _detached: bool = False

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_daemon("daemon transport is required")
        if not self._status.handle_id:
            raise _invalid_daemon("handle_id is required")

    @property
    def handle_id(self) -> str:
        return self._status.handle_id

    @property
    def state(self) -> DaemonLifecycleState:
        return self._status.state

    @property
    def endpoints(self) -> Endpoints:
        return self._status.endpoints

    def status(self) -> DaemonStatus:
        self._require_attached()
        try:
            raw = self.transport.status(self.handle_id)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("daemon status failed", exc) from exc
        self._status = DaemonStatus.from_json(raw)
        return self._status

    def open_runtime(self, options: ConnectOptions = ConnectOptions()) -> RuntimeClient:
        self._require_attached()
        if not _runtime_ready(self.state):
            raise _invalid_daemon("daemon invocation endpoint is not ready")
        try:
            transport, _facts = self.transport.open_runtime(
                self.handle_id, options.to_json_bytes()
            )
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("daemon open runtime failed", exc) from exc
        if transport is None:
            raise _invalid_daemon("runtime transport is required")
        return RuntimeClient(transport)

    def stop(self, options: StopOptions = StopOptions()) -> None:
        self._require_attached()
        if self.state == DaemonLifecycleState.STOPPED:
            return
        try:
            raw = self.transport.stop(self.handle_id, options.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("daemon stop failed", exc) from exc
        self._status = DaemonStatus.from_json(raw)

    def detach(self) -> None:
        if self._detached:
            return
        if self.transport is None:
            raise _invalid_daemon("daemon handle is not initialized")
        try:
            self.transport.detach(self.handle_id)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("daemon detach failed", exc) from exc
        self._detached = True

    def _require_attached(self) -> None:
        if self.transport is None:
            raise _invalid_daemon("daemon handle is not initialized")
        if self._detached:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="sdk",
                retry=RetryHint.NEVER,
                retryable=False,
                message="daemon handle is detached",
            )


def start_daemon(transport: DaemonTransport, config: StartConfig) -> DaemonHandle:
    """Start a daemon through an explicit lifecycle transport seam."""

    return DaemonControl(transport).start(config)


def attach_daemon(
    transport: DaemonTransport, options: AttachOptions = AttachOptions()
) -> DaemonHandle:
    """Attach to an existing daemon through an explicit lifecycle transport seam."""

    return DaemonControl(transport).attach(options)


def discover_daemon(
    transport: DaemonTransport, options: DiscoverOptions = DiscoverOptions()
) -> Endpoints:
    """Return daemon-advertised endpoints through an explicit transport seam."""

    return DaemonControl(transport).discover(options)


def connect_local(
    transport: DaemonTransport, options: ConnectOptions = ConnectOptions()
) -> RuntimeClient:
    """Discover, attach, open, and detach a local daemon runtime."""

    return DaemonControl(transport).connect_local(options)


def _validate_start_config(config: StartConfig) -> None:
    if config.mode == DaemonMode.DEVICE and config.listen_tcp:
        raise _invalid_daemon("device mode must not accept a public TCP listener")
    if (
        config.mode in {DaemonMode.HUB, DaemonMode.BOTH}
        and config.listen_tcp
        and (not config.tls_cert_path or not config.tls_key_path)
    ):
        raise _invalid_daemon("public TCP listener requires TLS material")


def _require_runtime_ready(status: DaemonStatus) -> None:
    if _runtime_ready(status.state):
        return
    if status.state in {
        DaemonLifecycleState.CONTROL_ONLY,
        DaemonLifecycleState.CONTROL_READY,
    }:
        raise SDKError(
            code=ErrorCode.CONTROL_ONLY,
            stage="daemon_lifecycle",
            retry=RetryHint.SAFE,
            retryable=True,
            message="daemon control endpoint is ready but invocation endpoint is not ready",
        )
    raise _invalid_daemon("daemon invocation endpoint is not ready")


def _runtime_ready(state: DaemonLifecycleState) -> bool:
    return state in {
        DaemonLifecycleState.INVOCATION_READY,
        DaemonLifecycleState.RUNNING,
    }


def _json_bytes(value: Mapping[str, object]) -> bytes:
    compact = {key: item for key, item in value.items() if item not in ("", 0, False)}
    return json.dumps(compact, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _json_object(raw: bytes | str, label: str) -> dict[str, object]:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_daemon(f"decode {label} JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_daemon(f"{label} JSON must be an object")
    return decoded


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_daemon(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_daemon(f"{field_name} must be a string or null")
    return value


def _optional_non_negative_int(value: object, field_name: str) -> int:
    if value is None:
        return 0
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_daemon(f"{field_name} must be a non-negative integer")
    return value


def _invalid_daemon(
    message: str, cause: BaseException | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="sdk",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )


def _transport_error(message: str, cause: BaseException) -> SDKError:
    return SDKError(
        code=ErrorCode.TRANSPORT,
        stage="transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=message,
        cause=cause,
    )
