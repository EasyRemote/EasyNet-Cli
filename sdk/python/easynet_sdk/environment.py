"""Process-level SDK environment root."""

from __future__ import annotations

from dataclasses import dataclass, field
from threading import Lock
from typing import Protocol, TypeVar

from .ability_invocation import AbilityInvocationClient
from .client import Client, FeatureSet
from .connection import (
    ConnectOptions,
    ControlDiscoveryRuntimeConnector,
    RuntimeConnection,
)
from .control_ipc import ControlIpcClient, default_control_path
from .daemon import DaemonControl
from .errors import ErrorCode, RetryHint, SDKError
from .health import HealthClient
from .axon_addressing import AddressingClient
from .runtime import RuntimeClient
from .transport import DaemonInvocationTransport


class _Closable(Protocol):
    def close(self) -> None:
        ...


_TClosable = TypeVar("_TClosable", bound=_Closable)


@dataclass
class NativeRuntimeHandle:
    """One SDK-owned native Runtime and Health provider lifecycle.

    Health borrows Runtime's transport. Addressing is an Axon-backed local
    provider and never depends on a product profile exported by generic ABI v5.
    """

    _runtime: RuntimeClient
    _health: HealthClient
    _addressing: AddressingClient
    _closed: bool = field(default=False, init=False, repr=False)
    _lock: Lock = field(default_factory=Lock, init=False, repr=False)

    def __post_init__(self) -> None:
        for name, value in (
            ("runtime", self._runtime),
            ("health", self._health),
            ("addressing", self._addressing),
        ):
            if value is None:
                raise SDKError(
                    code=ErrorCode.INVALID_ARGUMENT,
                    stage="sdk",
                    retry=RetryHint.NEVER,
                    retryable=False,
                    message=f"native runtime {name} provider is required",
                )

    def client(self) -> RuntimeClient:
        """Return the provider's Runtime Core facade."""

        self._require_open()
        return self._runtime

    def health(self) -> HealthClient:
        """Return the provider's borrowed Health facade."""

        self._require_open()
        return self._health

    def addressing(self) -> AddressingClient:
        """Return the provider-backed product-neutral Addressing facade."""

        self._require_open()
        return self._addressing

    def close(self) -> None:
        """Close all provider-owned facades exactly once."""

        with self._lock:
            if self._closed:
                return
            self._closed = True
        first_error: SDKError | None = None
        for client in (self._health, self._addressing, self._runtime):
            if client is None:
                continue
            try:
                client.close()
            except SDKError as exc:
                if first_error is None:
                    first_error = exc
            except Exception as exc:
                if first_error is None:
                    first_error = SDKError(
                        code=ErrorCode.ROUTE_UNAVAILABLE,
                        stage="sdk",
                        retry=RetryHint.SAFE,
                        retryable=True,
                        message="native runtime provider close failed",
                        cause=exc,
                    )
        if first_error is not None:
            raise first_error

    def _require_open(self) -> None:
        with self._lock:
            if self._closed:
                raise SDKError(
                    code=ErrorCode.INVALID_ARGUMENT,
                    stage="sdk",
                    retry=RetryHint.NEVER,
                    retryable=False,
                    message="native runtime provider is closed",
                )


@dataclass
class SdkEnvironment:
    """Public process root for SDK initialization and default clients."""

    library_path: str | None = None
    control_path: str = ""
    _closed: bool = field(default=False, init=False, repr=False)
    _owned: list[_Closable] = field(default_factory=list, init=False, repr=False)

    def feature_set(self) -> FeatureSet:
        """Return daemon SDK feature discovery for the configured library."""

        client = self.client()
        try:
            return client.require_abi(_expected_abi_version())
        finally:
            client.close()

    def client(self) -> Client:
        """Open the public feature-discovery client."""

        self._require_open()
        from . import _cabi

        transport = _cabi.CABIDiscoveryTransport(
            _cabi.CLILibrary.load(self.library_path)
        )
        return self._track(Client(transport))

    def daemon_control(self) -> DaemonControl:
        """Open daemon lifecycle control over the SDK default transport."""

        self._require_open()
        from . import _cabi

        transport = _cabi.open_cabi_daemon_transport(
            library_path=self.library_path,
        )
        self._track(transport)
        return DaemonControl(transport)

    def control_ipc_client(self, *, timeout: float | None = None) -> ControlIpcClient:
        """Open a direct boot/status control IPC client."""

        self._require_open()
        return self._track(
            ControlIpcClient.connect(
                self.resolved_control_path(),
                timeout=timeout,
            )
        )

    def connect_local(
        self, options: ConnectOptions = ConnectOptions()
    ) -> RuntimeClient:
        """Discover, attach, open, and detach a local daemon runtime client."""

        client = self.daemon_control().connect_local(self._connect_options(options))
        return self._track(client)

    def runtime_connection(
        self, options: ConnectOptions = ConnectOptions()
    ) -> RuntimeConnection:
        """Open a stateful RuntimeConnection over the SDK default connector."""

        self._require_open()
        from . import _cabi

        options = self._connect_options(options)
        connector = ControlDiscoveryRuntimeConnector(
            _cabi.open_cabi_runtime_connector(library_path=self.library_path),
            control_path=options.control_path,
        )
        connection = RuntimeConnection(connector)
        connection.connect(options)
        return self._track(connection)

    def runtime_connection_direct(
        self, options: ConnectOptions = ConnectOptions()
    ) -> RuntimeConnection:
        """Open a direct Axon runtime connection with canonical Addressing."""

        self._require_open()
        from .direct_runtime import DirectDaemonRuntimeConnector

        addressing = _canonical_addressing_client()
        connector = DirectDaemonRuntimeConnector(
            control_path=self.resolved_control_path(),
            identity=addressing,
            close_identity=True,
        )
        connection = RuntimeConnection(connector)
        connection.connect(self._connect_options(options))
        return self._track(connection)

    def runtime_client(self) -> RuntimeClient:
        """Open a direct runtime client for the configured control path."""

        self._require_open()
        from . import _cabi

        transport = _cabi.open_cabi_runtime_transport(
            control_path=self.resolved_control_path(),
            library_path=self.library_path,
        )
        return self._track(RuntimeClient(transport))

    def native_runtime(
        self, options: ConnectOptions = ConnectOptions()
    ) -> NativeRuntimeHandle:
        """Open one owned native Runtime and Health provider."""

        self._require_open()
        from . import _cabi

        resolved = self._connect_options(options)
        runtime_transport = _cabi.open_cabi_runtime_transport(
            control_path=resolved.control_path,
            library_path=self.library_path,
        )
        runtime = RuntimeClient(runtime_transport)
        health = HealthClient(runtime_transport, owns_transport=False)
        addressing = _canonical_addressing_client()
        return self._track(NativeRuntimeHandle(runtime, health, addressing))

    def runtime_client_direct(
        self, options: ConnectOptions = ConnectOptions()
    ) -> RuntimeClient:
        """Open a direct daemon Axon gRPC-over-UDS runtime client."""

        return self._track(self.runtime_connection_direct(options).runtime_client())

    def invocation_transport(self) -> DaemonInvocationTransport:
        """Open the public JSON-friendly daemon Invocation transport facade."""

        self._require_open()
        return self._track(
            DaemonInvocationTransport.connect(
                control_path=self.resolved_control_path(),
                library_path=self.library_path,
            )
        )

    def invocation_transport_direct(
        self, options: ConnectOptions = ConnectOptions()
    ) -> DaemonInvocationTransport:
        """Open the JSON-friendly daemon Invocation facade over direct UDS."""

        self._require_open()
        return self._track(
            DaemonInvocationTransport.connect_direct(
                control_path=self.resolved_control_path(),
                library_path=self.library_path,
                options=self._connect_options(options),
            )
        )

    def ability_invocation_client(self) -> AbilityInvocationClient:
        """Open the generic complete-Invocation facade."""

        self._require_open()
        return self._track(
            AbilityInvocationClient(self.runtime_client(), _canonical_addressing_client())
        )

    def health_client(self) -> HealthClient:
        """Open a health facade for the configured control path."""

        self._require_open()
        from . import _cabi

        transport = _cabi.open_cabi_runtime_transport(
            control_path=self.resolved_control_path(),
            library_path=self.library_path,
        )
        self._track(transport)
        return HealthClient(transport)

    def addressing_client(self) -> AddressingClient:
        """Open the product-neutral Axon-backed Addressing provider."""

        self._require_open()
        return self._track(_canonical_addressing_client())

    def close(self) -> None:
        """Close SDK-owned resources without stopping daemon processes."""

        if self._closed:
            return
        self._closed = True
        first_error: SDKError | None = None
        while self._owned:
            owned = self._owned.pop()
            try:
                owned.close()
            except SDKError as exc:
                if first_error is None:
                    first_error = exc
            except Exception as exc:
                if first_error is None:
                    first_error = SDKError(
                        code=ErrorCode.ROUTE_UNAVAILABLE,
                        stage="sdk",
                        retry=RetryHint.SAFE,
                        retryable=True,
                        message="SDK environment close failed",
                        cause=exc,
                    )
        if first_error is not None:
            raise first_error

    def _track(self, owned: _TClosable) -> _TClosable:
        self._owned.append(owned)
        return owned

    def resolved_control_path(self) -> str:
        """Return the configured daemon control discovery path or SDK default."""

        self._require_open()
        return self.control_path or str(default_control_path())

    def _connect_options(self, options: ConnectOptions) -> ConnectOptions:
        control_path = options.control_path or self.resolved_control_path()
        return ConnectOptions(
            endpoint=options.endpoint,
            control_path=control_path,
            dial_timeout_ms=options.dial_timeout_ms,
            invoke_timeout_ms=options.invoke_timeout_ms,
            max_message_bytes=options.max_message_bytes,
            reconnect=options.reconnect,
        )

    def _profile_transport(self, opener: object) -> _Closable:
        self._require_open()
        return opener(
            control_path=self.resolved_control_path(),
            library_path=self.library_path,
        )

    def _require_open(self) -> None:
        if self._closed:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="sdk",
                retry=RetryHint.NEVER,
                retryable=False,
                message="SDK environment is closed",
            )


def default_environment(
    *, library_path: str | None = None, control_path: str = ""
) -> SdkEnvironment:
    """Create the default public SDK process root."""

    return SdkEnvironment(library_path=library_path, control_path=control_path)


def _expected_abi_version() -> int:
    from . import _cabi

    return _cabi.EXPECTED_ABI_VERSION


def _canonical_addressing_client() -> AddressingClient:
    from .axon_addressing import AxonAddressingTransport

    return AddressingClient(AxonAddressingTransport())
