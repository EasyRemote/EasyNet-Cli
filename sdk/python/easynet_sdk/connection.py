"""Runtime Core connection state facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Callable, Mapping, Protocol, runtime_checkable

from .providers.runtime.control import _ControlDiscovery, _read_control_discovery
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
    """Runtime Core connection knobs."""

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


def _connect_options_or_default(options: ConnectOptions | None) -> ConnectOptions:
    """Materialize omitted connection options at the call boundary."""

    return options if options is not None else ConnectOptions()


@dataclass(frozen=True)
class RuntimeEndpoint:
    """Resolved runtime invocation endpoint projection."""

    endpoint: str
    control_path: str = ""
    protocol_version: str = ""
    abi_version: int = 0

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
            protocol_version=_optional_string(
                decoded.get("protocol_version"), "protocol_version"
            )
            or "",
            abi_version=_optional_non_negative_int(
                decoded.get("abi_version"), "abi_version"
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
class _ControlDiscoveryRuntimeConnector:
    """Resolve Runtime Core endpoints from runtime-host control discovery.

    The connector owns only discovery-to-endpoint projection. Handshake,
    transport lifetime, and Invocation protocol behavior stay delegated to the
    inner connector, usually the private generic C ABI v8 connector.
    """

    inner: RuntimeConnector
    control_path: str = ""
    discovery_reader: Callable[[str], _ControlDiscovery] = _read_control_discovery
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

    def connect(self, options: ConnectOptions | None = None) -> None:
        if self.state == ConnectionState.CLOSED:
            raise _invalid_connection("runtime connection is closed")
        options = _connect_options_or_default(options)
        options_json = options.to_json_bytes()
        attempts = 2 if options.reconnect else 1
        for attempt in range(attempts):
            self._transition(ConnectionState.RESOLVING)
            try:
                self._connect_attempt(options_json)
                return
            except SDKError as exc:
                error = exc
            except Exception as exc:
                error = _transport_error("runtime connection failed", exc)
            self.last_error = error
            if attempt + 1 == attempts:
                self._fail(error)
                raise error
            self._transition(ConnectionState.DEGRADED)
            self._transition(ConnectionState.RECONNECTING)

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
            self._transition(ConnectionState.CLOSED)
            self.last_error = exc
            raise
        except Exception as exc:
            self._transport = None
            self._transition(ConnectionState.CLOSED)
            error = _transport_error("runtime close failed", exc)
            self.last_error = error
            raise error from exc
        self._transport = None
        self.last_error = None
        self._transition(ConnectionState.CLOSED)

    def _connect_attempt(self, options_json: bytes) -> None:
        endpoint_json = self.connector.resolve(options_json)
        endpoint = RuntimeEndpoint.from_json(endpoint_json)
        self._transition(ConnectionState.CONNECTING)
        transport, handshake_json = self.connector.handshake(endpoint_json)
        if transport is None:
            raise _invalid_connection("runtime transport is required after handshake")
        self.endpoint = endpoint
        self._transport = transport
        self.handshake_facts = _handshake_facts(handshake_json)
        self.last_error = None
        self._transition(ConnectionState.READY)

    def _fail(self, error: SDKError) -> None:
        self._transport = None
        self.last_error = error
        if self.state != ConnectionState.CLOSED:
            self._transition(ConnectionState.FAILED)

    def _transition(self, next_state: ConnectionState) -> None:
        allowed = {
            ConnectionState.IDLE: {
                ConnectionState.RESOLVING,
                ConnectionState.CLOSED,
            },
            ConnectionState.RESOLVING: {
                ConnectionState.CONNECTING,
                ConnectionState.DEGRADED,
                ConnectionState.FAILED,
                ConnectionState.CLOSED,
            },
            ConnectionState.CONNECTING: {
                ConnectionState.READY,
                ConnectionState.DEGRADED,
                ConnectionState.FAILED,
                ConnectionState.CLOSED,
            },
            ConnectionState.READY: {
                ConnectionState.RESOLVING,
                ConnectionState.CLOSED,
            },
            ConnectionState.DEGRADED: {
                ConnectionState.RECONNECTING,
                ConnectionState.FAILED,
                ConnectionState.CLOSED,
            },
            ConnectionState.RECONNECTING: {
                ConnectionState.RESOLVING,
                ConnectionState.FAILED,
                ConnectionState.CLOSED,
            },
            ConnectionState.FAILED: {
                ConnectionState.RESOLVING,
                ConnectionState.CLOSED,
            },
            ConnectionState.CLOSED: {ConnectionState.CLOSED},
        }
        if next_state not in allowed.get(self.state, set()):
            raise _invalid_connection(
                "runtime connection cannot transition "
                f"from {self.state.value} to {next_state.value}"
            )
        self.state = next_state


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
        code=ErrorCode.ROUTE_UNAVAILABLE,
        stage="transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=message,
        cause=cause,
    )
