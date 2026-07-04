"""Runtime Core connection state facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Callable, Mapping, Protocol, runtime_checkable

from .control_ipc import ControlDiscovery, read_control_discovery
from .errors import ErrorCode, RetryHint, SDKError
from .runtime import RuntimeClient, RuntimeTransport


class ConnectionState(StrEnum):
    """Runtime Core client connection states."""

    IDLE = "Idle"
    RESOLVING = "Resolving"
    CONNECTING = "Connecting"
    READY = "Ready"
    DEGRADED = "Degraded"
    RECONNECTING = "Reconnecting"
    FAILED = "Failed"
    CLOSED = "Closed"


@dataclass(frozen=True)
class ConnectOptions:
    """Daemon Runtime Core connection knobs."""

    endpoint: str = ""
    control_path: str = ""
    dial_timeout_ms: int = 0
    invoke_timeout_ms: int = 0
    max_message_bytes: int = 0
    reconnect: bool = False

    def to_json_dict(self) -> dict[str, object]:
        value: dict[str, object] = {}
        if self.endpoint:
            value["endpoint"] = self.endpoint
        if self.control_path:
            value["control_path"] = self.control_path
        if self.dial_timeout_ms:
            value["dial_timeout_ms"] = self.dial_timeout_ms
        if self.invoke_timeout_ms:
            value["invoke_timeout_ms"] = self.invoke_timeout_ms
        if self.max_message_bytes:
            value["max_message_bytes"] = self.max_message_bytes
        if self.reconnect:
            value["reconnect"] = self.reconnect
        return value

    def to_json_bytes(self) -> bytes:
        return json.dumps(
            self.to_json_dict(), separators=(",", ":"), sort_keys=True
        ).encode("utf-8")


@dataclass(frozen=True)
class RuntimeEndpoint:
    """Resolved daemon invocation endpoint projection."""

    endpoint: str
    control_path: str = ""
    control_endpoint: str = ""
    protocol_version: str = ""
    abi_version: int = 0
    daemon_version: str = ""
    capability_flags: tuple[str, ...] = ()

    @classmethod
    def from_json(cls, raw: bytes | str) -> "RuntimeEndpoint":
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_connection(f"decode runtime endpoint JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_connection("runtime endpoint JSON must be an object")
        endpoint = _required_string(decoded, "endpoint")
        return cls(
            endpoint=endpoint,
            control_path=_optional_string(decoded.get("control_path"), "control_path")
            or "",
            control_endpoint=_optional_string(
                decoded.get("control_endpoint"), "control_endpoint"
            )
            or "",
            protocol_version=_optional_string(
                decoded.get("protocol_version"), "protocol_version"
            )
            or "",
            abi_version=_optional_non_negative_int(
                decoded.get("abi_version"), "abi_version"
            ),
            daemon_version=_optional_string(
                decoded.get("daemon_version"), "daemon_version"
            )
            or "",
            capability_flags=_string_tuple(
                decoded.get("capability_flags", []), "capability_flags"
            ),
        )


@runtime_checkable
class RuntimeConnector(Protocol):
    """Concrete connection steps supplied by the integration layer."""

    def resolve(self, options_json: bytes) -> bytes:
        ...

    def handshake(self, endpoint_json: bytes) -> tuple[RuntimeTransport, bytes]:
        ...

    def close(self) -> None:
        ...


@dataclass
class ControlDiscoveryRuntimeConnector:
    """Resolve Runtime Core endpoints from daemon control discovery.

    The connector owns only discovery-to-endpoint projection. Handshake,
    transport lifetime, and Invocation protocol behavior stay delegated to the
    inner connector, usually the private C ABI v4 connector.
    """

    inner: RuntimeConnector
    control_path: str = ""
    discovery_reader: Callable[[str], ControlDiscovery] = read_control_discovery
    _closed: bool = False

    def __post_init__(self) -> None:
        if self.inner is None:
            raise _invalid_connection("inner runtime connector is required")

    def resolve(self, options_json: bytes) -> bytes:
        self._require_open()
        options = _connect_options(options_json)
        option_endpoint = _optional_string(options.get("endpoint"), "endpoint") or ""
        option_control_path = (
            _optional_string(options.get("control_path"), "control_path") or ""
        )
        control_path = option_control_path or self.control_path
        if option_endpoint:
            return _json_bytes(
                {
                    "endpoint": option_endpoint,
                    "control_path": control_path,
                }
            )
        discovery = self.discovery_reader(control_path)
        if not discovery.invocation_endpoint:
            raise SDKError(
                code=ErrorCode.CONTROL_ONLY,
                stage="connection",
                retry=RetryHint.SAFE,
                retryable=True,
                message="control discovery did not advertise invocation_endpoint",
                details={"control_path": control_path},
            )
        return _json_bytes(
            {
                "endpoint": discovery.invocation_endpoint,
                "control_path": control_path,
                "control_endpoint": discovery.socket_path,
                "daemon_version": discovery.daemon_version,
                "capability_flags": list(discovery.capability_flags),
            }
        )

    def handshake(self, endpoint_json: bytes) -> tuple[RuntimeTransport, bytes]:
        self._require_open()
        return self.inner.handshake(endpoint_json)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        self.inner.close()

    def _require_open(self) -> None:
        if self._closed:
            raise _invalid_connection("runtime connector is closed")


@dataclass
class RuntimeConnection:
    """Runtime Core connection state object."""

    connector: RuntimeConnector
    state: ConnectionState = ConnectionState.IDLE
    endpoint: RuntimeEndpoint | None = None
    handshake_facts: Mapping[str, object] = field(default_factory=dict)
    last_error: SDKError | None = None
    _transport: RuntimeTransport | None = None

    def __post_init__(self) -> None:
        if self.connector is None:
            raise _invalid_connection("runtime connector is required")

    def connect(self, options: ConnectOptions = ConnectOptions()) -> None:
        if self.state == ConnectionState.CLOSED:
            raise _invalid_connection("runtime connection is closed")
        self.state = ConnectionState.RESOLVING
        try:
            endpoint_json = self.connector.resolve(options.to_json_bytes())
            endpoint = RuntimeEndpoint.from_json(endpoint_json)
            self.state = ConnectionState.CONNECTING
            transport, handshake_json = self.connector.handshake(endpoint_json)
            if transport is None:
                raise _invalid_connection("runtime transport is required after handshake")
            self.endpoint = endpoint
            self._transport = transport
            self.handshake_facts = _handshake_facts(handshake_json)
            self.last_error = None
            self.state = ConnectionState.READY
        except SDKError as exc:
            self._fail(exc)
            raise
        except Exception as exc:
            error = _transport_error("runtime connection failed", exc)
            self._fail(error)
            raise error from exc

    def runtime_client(self) -> RuntimeClient:
        if self.state != ConnectionState.READY or self._transport is None:
            raise _invalid_connection("runtime connection is not ready")
        return RuntimeClient(self._transport)

    def close(self) -> None:
        if self.state == ConnectionState.CLOSED:
            return
        try:
            self.connector.close()
        except SDKError as exc:
            self._transport = None
            self.state = ConnectionState.CLOSED
            self.last_error = exc
            raise
        except Exception as exc:
            self._transport = None
            self.state = ConnectionState.CLOSED
            error = _transport_error("runtime close failed", exc)
            self.last_error = error
            raise error from exc
        self._transport = None
        self.last_error = None
        self.state = ConnectionState.CLOSED

    def _fail(self, error: SDKError) -> None:
        self._transport = None
        self.last_error = error
        if self.state != ConnectionState.CLOSED:
            self.state = ConnectionState.FAILED


def _handshake_facts(raw: bytes) -> Mapping[str, object]:
    if raw == b"":
        return {}
    try:
        decoded = json.loads(raw)
    except Exception as exc:
        raise _invalid_connection(f"decode runtime handshake JSON: {exc}", exc) from exc
    if decoded is None:
        return {}
    if not isinstance(decoded, dict):
        raise _invalid_connection("runtime handshake JSON must be an object")
    return dict(decoded)


def _connect_options(raw: bytes) -> Mapping[str, object]:
    if raw == b"":
        return {}
    try:
        decoded = json.loads(raw)
    except Exception as exc:
        raise _invalid_connection(
            f"decode runtime connect options JSON: {exc}", exc
        ) from exc
    if decoded is None:
        return {}
    if not isinstance(decoded, dict):
        raise _invalid_connection("runtime connect options JSON must be an object")
    return decoded


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_connection(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_connection(f"{field_name} must be a string or null")
    return value


def _optional_non_negative_int(value: object, field_name: str) -> int:
    if value is None:
        return 0
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_connection(f"{field_name} must be a non-negative integer")
    return value


def _string_tuple(value: object, field_name: str) -> tuple[str, ...]:
    if value is None:
        return ()
    if not isinstance(value, list) or not all(
        isinstance(item, str) for item in value
    ):
        raise _invalid_connection(f"{field_name} must be an array of strings")
    return tuple(value)


def _invalid_connection(
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
