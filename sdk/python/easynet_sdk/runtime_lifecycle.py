"""Provider-neutral runtime host lifecycle state and handle ownership."""

from __future__ import annotations

import json
from dataclasses import dataclass, field, replace
from enum import StrEnum
from typing import TYPE_CHECKING, Any, Mapping, Protocol, runtime_checkable

from .connection import ConnectOptions, _connect_options_or_default
from .errors import ErrorCode, RetryHint, SDKError
from .runtime import RuntimeClient, RuntimeTransport

if TYPE_CHECKING:
    from .transport import InvocationResultAdapter


class RuntimeLifecycleState(StrEnum):
    """SDK runtime host lifecycle state projection."""

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


@runtime_checkable
class RuntimeHostStartRequest(Protocol):
    """Provider-owned start request accepted by the canonical host state machine."""

    def validate(self) -> None: ...

    def to_json_bytes(self) -> bytes: ...


@dataclass(frozen=True)
class AttachOptions:
    """Existing runtime host attachment request."""

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


@runtime_checkable
class RuntimeHostDiscoverRequest(Protocol):
    """Provider-owned endpoint discovery request."""

    def to_json_bytes(self) -> bytes: ...


@dataclass(frozen=True)
class RuntimeHostDiscoverOptions:
    """Provider-neutral discovery request for explicit endpoint locations."""

    control_endpoint: str = ""
    control_path: str = ""

    def to_json_bytes(self) -> bytes:
        return _json_bytes(
            {
                "control_endpoint": self.control_endpoint,
                "control_path": self.control_path,
            }
        )


@dataclass(frozen=True)
class StopOptions:
    """Runtime host stop policy."""

    graceful_timeout_ms: int = 0
    force: bool = False

    def to_json_bytes(self) -> bytes:
        return _json_bytes(
            {
                "graceful_timeout_ms": self.graceful_timeout_ms,
                "force": self.force,
            }
        )


def _attach_options_or_default(options: AttachOptions | None) -> AttachOptions:
    """Materialize omitted attach options at the lifecycle boundary."""

    return options if options is not None else AttachOptions()


def _discover_options_or_default(
    options: RuntimeHostDiscoverRequest | None,
) -> RuntimeHostDiscoverRequest:
    """Materialize omitted discover options at the lifecycle boundary."""

    return options if options is not None else RuntimeHostDiscoverOptions()


def _stop_options_or_default(options: StopOptions | None) -> StopOptions:
    """Materialize omitted stop options at the lifecycle boundary."""

    return options if options is not None else StopOptions()


@dataclass(frozen=True)
class Endpoints:
    """Runtime host control and Invocation transport locators."""

    control_endpoint: str = ""
    invocation_endpoint: str = ""
    public_endpoint: str = ""

    @classmethod
    def from_json(
        cls, raw: bytes | str, *, require_invocation: bool = True
    ) -> "Endpoints":
        decoded = _json_object(raw, "runtime host endpoints")
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
            raise _invalid_runtime_lifecycle("invocation_endpoint is required")
        return endpoint


@dataclass(frozen=True)
class RuntimeStatus:
    """Typed runtime host lifecycle status projection."""

    state: RuntimeLifecycleState
    handle_id: str = ""
    mode: str = ""
    pid: int = 0
    version: str = ""
    message: str = ""
    endpoints: Endpoints = field(default_factory=Endpoints)
    diagnostics: tuple[str, ...] = field(default_factory=tuple)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "RuntimeStatus":
        decoded = _json_object(raw, "runtime host status")
        raw_state = _required_string(decoded, "state")
        try:
            state = RuntimeLifecycleState(raw_state)
        except ValueError as exc:
            raise _invalid_runtime_lifecycle(
                "invalid runtime lifecycle state", exc
            ) from exc
        mode = _runtime_status_mode(decoded.get("mode"))
        endpoints_value = decoded.get("endpoints", {})
        if endpoints_value is None:
            endpoints_value = {}
        if not isinstance(endpoints_value, dict):
            raise _invalid_runtime_lifecycle("endpoints must be an object")
        diagnostics_value = decoded.get("diagnostics", [])
        if not isinstance(diagnostics_value, list) or not all(
            isinstance(item, str) for item in diagnostics_value
        ):
            raise _invalid_runtime_lifecycle("diagnostics must be an array of strings")
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


def _runtime_status_mode(value: object) -> str:
    mode = _optional_string(value, "mode") or ""
    if mode and mode not in {"edge", "authority", "combined"}:
        raise _invalid_runtime_lifecycle("invalid runtime host mode")
    return mode


@runtime_checkable
class RuntimeLifecycleTransport(Protocol):
    """Concrete runtime host lifecycle operations supplied by integration."""

    def discover(self, options_json: bytes) -> bytes: ...

    def start(self, config_json: bytes) -> bytes: ...

    def attach(self, options_json: bytes) -> bytes: ...

    def status(self, handle_id: str) -> bytes: ...

    def invocation_endpoint(self, handle_id: str) -> str: ...

    def open_runtime(
        self, handle_id: str, options_json: bytes
    ) -> tuple[RuntimeTransport, bytes]: ...

    def stop(self, handle_id: str, options_json: bytes) -> bytes: ...

    def detach(self, handle_id: str) -> None: ...


@dataclass(frozen=True)
class RuntimeLifecycle:
    """Runtime host lifecycle facade root over an integration transport."""

    transport: RuntimeLifecycleTransport

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_runtime_lifecycle("runtime lifecycle transport is required")

    def discover(
        self,
        options: RuntimeHostDiscoverRequest | None = None,
    ) -> Endpoints:
        options = _discover_options_or_default(options)
        try:
            raw = self.transport.discover(options.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("runtime host discover failed", exc) from exc
        return Endpoints.from_json(raw)

    def start(self, config: RuntimeHostStartRequest) -> "RuntimeHandle":
        if config is None or not isinstance(config, RuntimeHostStartRequest):
            raise _invalid_runtime_lifecycle("runtime host start request is required")
        config.validate()
        try:
            raw = self.transport.start(config.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("runtime host start failed", exc) from exc
        status = RuntimeStatus.from_json(raw)
        _require_runtime_ready(status)
        return RuntimeHandle(self.transport, status)

    def attach(self, options: AttachOptions | None = None) -> "RuntimeHandle":
        options = _attach_options_or_default(options)
        try:
            raw = self.transport.attach(options.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("runtime host attach failed", exc) from exc
        status = RuntimeStatus.from_json(raw)
        _require_runtime_ready(status)
        return RuntimeHandle(self.transport, status)

    def connect_local(
        self, options: ConnectOptions | None = None
    ) -> RuntimeClient:
        options = _connect_options_or_default(options)
        endpoints = self.discover(
            RuntimeHostDiscoverOptions(control_path=options.control_path)
        )
        runtime_endpoint = options.endpoint or endpoints.invocation_endpoint
        if not runtime_endpoint:
            raise _invalid_runtime_lifecycle("invocation_endpoint is required")
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


@dataclass(frozen=True)
class RuntimeHost:
    """Host-facing runtime lifecycle facade over ``RuntimeLifecycle``."""

    control: RuntimeLifecycle

    def start(self, config: RuntimeHostStartRequest) -> "RuntimeHostHandle":
        if config is None:
            raise _invalid_runtime_lifecycle("runtime host start request is required")
        return RuntimeHostHandle(self.control.start(config))


@dataclass(frozen=True)
class RuntimeHostHandle:
    """Host-facing runtime handle projection."""

    handle: "RuntimeHandle"

    def status_dict(self) -> dict[str, Any]:
        status = self.handle.status()
        return {
            "state": status.state.value,
            "handle_id": status.handle_id,
            "mode": status.mode,
            "pid": status.pid,
            "version": status.version,
            "message": status.message,
            "endpoints": {
                "control_endpoint": status.endpoints.control_endpoint,
                "invocation_endpoint": status.endpoints.invocation_endpoint,
                "public_endpoint": status.endpoints.public_endpoint,
            },
            "diagnostics": list(status.diagnostics),
        }

    def invocation_endpoint(self) -> str:
        return self.handle.invocation_endpoint()

    def open_transport_adapter(self) -> "InvocationResultAdapter":
        from .transport import InvocationResultAdapter

        return InvocationResultAdapter.from_runtime_client(self.handle.open_runtime())

    def stop(self) -> None:
        self.handle.stop()


@dataclass
class RuntimeHandle:
    """Local runtime host lifecycle handle state."""

    transport: RuntimeLifecycleTransport
    _status: RuntimeStatus
    _detached: bool = False

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_runtime_lifecycle("runtime lifecycle transport is required")
        if not self._status.handle_id:
            raise _invalid_runtime_lifecycle("handle_id is required")

    @property
    def handle_id(self) -> str:
        return self._status.handle_id

    @property
    def state(self) -> RuntimeLifecycleState:
        return self._status.state

    @property
    def endpoints(self) -> Endpoints:
        return self._status.endpoints

    def invocation_endpoint(self) -> str:
        """Return the current runtime Invocation endpoint for this handle."""

        self._require_attached()
        if not _runtime_ready(self.state):
            raise _invalid_runtime_lifecycle("runtime invocation endpoint is not ready")
        try:
            endpoint = self.transport.invocation_endpoint(self.handle_id)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(
                "runtime invocation endpoint lookup failed", exc
            ) from exc
        if not endpoint:
            raise _invalid_runtime_lifecycle("invocation_endpoint is required")
        self._status = replace(
            self._status,
            endpoints=replace(
                self._status.endpoints,
                invocation_endpoint=endpoint,
            ),
        )
        return endpoint

    def status(self) -> RuntimeStatus:
        self._require_attached()
        try:
            raw = self.transport.status(self.handle_id)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("runtime host status failed", exc) from exc
        status = RuntimeStatus.from_json(raw)
        _validate_runtime_lifecycle_transition(self._status.state, status.state)
        self._status = status
        return self._status

    def open_runtime(self, options: ConnectOptions | None = None) -> RuntimeClient:
        self._require_attached()
        if not _runtime_ready(self.state):
            raise _invalid_runtime_lifecycle("runtime invocation endpoint is not ready")
        options = _connect_options_or_default(options)
        try:
            transport, _facts = self.transport.open_runtime(
                self.handle_id, options.to_json_bytes()
            )
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("runtime open failed", exc) from exc
        if transport is None:
            raise _invalid_runtime_lifecycle("runtime transport is required")
        return RuntimeClient(transport)

    def runtime(self, options: ConnectOptions | None = None) -> RuntimeClient:
        """Open a Runtime Core client from this runtime host handle."""

        return self.open_runtime(options)

    def stop(self, options: StopOptions | None = None) -> None:
        self._require_attached()
        if self.state == RuntimeLifecycleState.STOPPED:
            return
        options = _stop_options_or_default(options)
        try:
            raw = self.transport.stop(self.handle_id, options.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("runtime host stop failed", exc) from exc
        status = RuntimeStatus.from_json(raw)
        _validate_runtime_lifecycle_transition(self._status.state, status.state)
        self._status = status

    def detach(self) -> None:
        if self._detached:
            return
        if self.transport is None:
            raise _invalid_runtime_lifecycle("runtime handle is not initialized")
        try:
            self.transport.detach(self.handle_id)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("runtime host detach failed", exc) from exc
        self._detached = True

    def _require_attached(self) -> None:
        if self.transport is None:
            raise _invalid_runtime_lifecycle("runtime handle is not initialized")
        if self._detached:
            raise SDKError(
                code=ErrorCode.INVALID_HANDLE,
                stage="sdk",
                retry=RetryHint.NEVER,
                retryable=False,
                message="runtime handle is detached",
            )


def start_runtime_host(
    transport: RuntimeLifecycleTransport, config: RuntimeHostStartRequest
) -> RuntimeHandle:
    """Start a runtime host through an explicit lifecycle transport."""

    return RuntimeLifecycle(transport).start(config)


def attach_runtime_host(
    transport: RuntimeLifecycleTransport, options: AttachOptions | None = None
) -> RuntimeHandle:
    """Attach to an existing runtime host through an explicit lifecycle transport."""

    return RuntimeLifecycle(transport).attach(options)


def discover_runtime_host(
    transport: RuntimeLifecycleTransport,
    options: RuntimeHostDiscoverRequest | None = None,
) -> Endpoints:
    """Return runtime-host-advertised endpoints through an explicit transport."""

    return RuntimeLifecycle(transport).discover(options)


def connect_runtime_local(
    transport: RuntimeLifecycleTransport, options: ConnectOptions | None = None
) -> RuntimeClient:
    """Discover, attach, open, and detach a local runtime host."""

    return RuntimeLifecycle(transport).connect_local(options)


def _require_runtime_ready(status: RuntimeStatus) -> None:
    if _runtime_ready(status.state):
        return
    if status.state in {
        RuntimeLifecycleState.CONTROL_ONLY,
        RuntimeLifecycleState.CONTROL_READY,
    }:
        raise SDKError(
            code=ErrorCode.CONTROL_ONLY,
            stage="runtime_lifecycle",
            retry=RetryHint.SAFE,
            retryable=True,
            message=(
                "runtime control endpoint is ready but invocation endpoint is not ready"
            ),
        )
    raise _invalid_runtime_lifecycle("runtime invocation endpoint is not ready")


def _runtime_ready(state: RuntimeLifecycleState) -> bool:
    return state in {
        RuntimeLifecycleState.INVOCATION_READY,
        RuntimeLifecycleState.RUNNING,
    }


_VALID_RUNTIME_LIFECYCLE_TRANSITIONS: dict[
    RuntimeLifecycleState, frozenset[RuntimeLifecycleState]
] = {
    RuntimeLifecycleState.DISCOVERED: frozenset(
        {
            RuntimeLifecycleState.STARTING,
            RuntimeLifecycleState.CONTROL_READY,
            RuntimeLifecycleState.INVOCATION_READY,
            RuntimeLifecycleState.RUNNING,
            RuntimeLifecycleState.CONTROL_ONLY,
            RuntimeLifecycleState.START_FAILED,
        }
    ),
    RuntimeLifecycleState.STARTING: frozenset(
        {
            RuntimeLifecycleState.CONTROL_READY,
            RuntimeLifecycleState.INVOCATION_READY,
            RuntimeLifecycleState.RUNNING,
            RuntimeLifecycleState.CONTROL_ONLY,
            RuntimeLifecycleState.START_FAILED,
            RuntimeLifecycleState.CONFIG_INVALID,
            RuntimeLifecycleState.PERMISSION_DENIED,
            RuntimeLifecycleState.VERSION_MISMATCH,
        }
    ),
    RuntimeLifecycleState.CONTROL_READY: frozenset(
        {
            RuntimeLifecycleState.INVOCATION_READY,
            RuntimeLifecycleState.RUNNING,
            RuntimeLifecycleState.CONTROL_ONLY,
            RuntimeLifecycleState.STOPPING,
            RuntimeLifecycleState.START_FAILED,
        }
    ),
    RuntimeLifecycleState.INVOCATION_READY: frozenset(
        {
            RuntimeLifecycleState.RUNNING,
            RuntimeLifecycleState.CONTROL_ONLY,
            RuntimeLifecycleState.INVOCATION_DOWN,
            RuntimeLifecycleState.STOPPING,
            RuntimeLifecycleState.CRASH_LOOP,
        }
    ),
    RuntimeLifecycleState.RUNNING: frozenset(
        {
            RuntimeLifecycleState.CONTROL_ONLY,
            RuntimeLifecycleState.INVOCATION_DOWN,
            RuntimeLifecycleState.STOPPING,
            RuntimeLifecycleState.STOPPED,
            RuntimeLifecycleState.CRASH_LOOP,
        }
    ),
    RuntimeLifecycleState.CONTROL_ONLY: frozenset(
        {
            RuntimeLifecycleState.INVOCATION_READY,
            RuntimeLifecycleState.RUNNING,
            RuntimeLifecycleState.STOPPING,
            RuntimeLifecycleState.CRASH_LOOP,
        }
    ),
    RuntimeLifecycleState.INVOCATION_DOWN: frozenset(
        {
            RuntimeLifecycleState.INVOCATION_READY,
            RuntimeLifecycleState.RUNNING,
            RuntimeLifecycleState.CONTROL_ONLY,
            RuntimeLifecycleState.STOPPING,
            RuntimeLifecycleState.CRASH_LOOP,
        }
    ),
    RuntimeLifecycleState.STOPPING: frozenset(
        {
            RuntimeLifecycleState.STOPPED,
            RuntimeLifecycleState.CRASH_LOOP,
        }
    ),
}


def _validate_runtime_lifecycle_transition(
    current: RuntimeLifecycleState, next_state: RuntimeLifecycleState
) -> None:
    if current == next_state:
        return
    if next_state in _VALID_RUNTIME_LIFECYCLE_TRANSITIONS.get(current, frozenset()):
        return
    raise _invalid_runtime_lifecycle(
        f"runtime lifecycle cannot transition from {current.value} to {next_state.value}"
    )


def _json_bytes(value: Mapping[str, object]) -> bytes:
    compact = {key: item for key, item in value.items() if item not in ("", 0, False)}
    return json.dumps(compact, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _json_object(raw: bytes | str, label: str) -> dict[str, object]:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_runtime_lifecycle(f"decode {label} JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_runtime_lifecycle(f"{label} JSON must be an object")
    return decoded


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_runtime_lifecycle(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_runtime_lifecycle(f"{field_name} must be a string or null")
    return value


def _optional_non_negative_int(value: object, field_name: str) -> int:
    if value is None:
        return 0
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_runtime_lifecycle(f"{field_name} must be a non-negative integer")
    return value


def _invalid_runtime_lifecycle(
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
