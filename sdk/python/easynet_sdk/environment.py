"""Process-level SDK environment root."""

from __future__ import annotations

from dataclasses import dataclass, field
from threading import Lock
from typing import Protocol, TypeVar

from .ability_invocation import AbilityInvocationClient
from .client import Client, FeatureSet
from .admin import AdminClient
from .compatibility import CompatibilityClient, RuntimeCompatibilityTransport
from .connection import (
    ConnectOptions,
    ControlDiscoveryRuntimeConnector,
    RuntimeConnection,
)
from .control_ipc import ControlIpcClient, default_control_path
from .daemon import DaemonControl
from .directory import DirectoryClient
from .errors import ErrorCode, RetryHint, SDKError
from .events import EventClient
from .health import HealthClient
from .host_binding import HostBindingClient
from .identity import AddressingClient, IdentityClient
from .mission import MissionClient, RuntimeMissionTransport
from .publication import PublicationClient, RuntimePublicationTransport
from .receipt import ReceiptClient
from .runtime import RuntimeClient
from .surface import RuntimeSurfaceTransport, SurfaceClient
from .transport import DaemonInvocationTransport
from .wrappers import RuntimeWrapperTransport, WrapperClient


class _Closable(Protocol):
    def close(self) -> None:
        ...


_TClosable = TypeVar("_TClosable", bound=_Closable)


@dataclass
class NativeRuntimeHandle:
    """One SDK-owned native Runtime, Health, and Identity provider lifecycle.

    The three facades are generic canonical-runtime concepts. Health borrows
    Runtime's transport, while Identity owns its independent daemon profile
    transport. Callers receive facades without ownership transfer; closing the
    handle releases them in reverse dependency order exactly once.
    """

    _runtime: RuntimeClient
    _health: HealthClient
    _identity: IdentityClient
    _closed: bool = field(default=False, init=False, repr=False)
    _lock: Lock = field(default_factory=Lock, init=False, repr=False)

    def client(self) -> RuntimeClient:
        """Return the provider's Runtime Core facade."""

        self._require_open()
        return self._runtime

    def health(self) -> HealthClient:
        """Return the provider's borrowed Health facade."""

        self._require_open()
        return self._health

    def identity(self) -> IdentityClient:
        """Return the provider's canonical Identity facade."""

        self._require_open()
        return self._identity

    def close(self) -> None:
        """Close all provider-owned facades exactly once."""

        with self._lock:
            if self._closed:
                return
            self._closed = True
        first_error: SDKError | None = None
        for client in (self._health, self._identity, self._runtime):
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
        """Open a stateful RuntimeConnection over daemon Axon gRPC UDS."""

        self._require_open()
        from . import _cabi
        from .direct_runtime import DirectDaemonRuntimeConnector

        options = self._connect_options(options)
        identity = AddressingClient(
            _cabi.open_cabi_identity_transport(
                control_path=options.control_path,
                library_path=self.library_path,
            )
        )
        connection = RuntimeConnection(
            DirectDaemonRuntimeConnector(
                control_path=options.control_path,
                identity=identity,
                close_identity=True,
            )
        )
        connection.connect(options)
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
        """Open one owned native Runtime, Health, and Identity provider.

        This mirrors the Go SDK native provider shape. Runtime and Health share
        a transport; Identity opens the daemon's dedicated canonical identity
        profile using the same resolved control endpoint and library options.
        """

        self._require_open()
        from . import _cabi

        resolved = self._connect_options(options)
        runtime_transport = _cabi.open_cabi_runtime_transport(
            control_path=resolved.control_path,
            library_path=self.library_path,
        )
        runtime = RuntimeClient(runtime_transport)
        health = HealthClient(runtime_transport, owns_transport=False)
        try:
            identity = IdentityClient(
                _cabi.open_cabi_identity_transport(
                    control_path=resolved.control_path,
                    library_path=self.library_path,
                )
            )
        except Exception:
            health.close()
            runtime.close()
            raise
        return self._track(NativeRuntimeHandle(runtime, health, identity))

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
        """Open the ability Invocation convenience facade."""

        self._require_open()
        from . import _cabi

        control_path = self.resolved_control_path()
        runtime_transport = _cabi.open_cabi_runtime_transport(
            control_path=control_path,
            library_path=self.library_path,
        )
        identity_transport = _cabi.open_cabi_identity_transport(
            control_path=control_path,
            library_path=self.library_path,
        )
        return self._track(
            AbilityInvocationClient(
                runtime=RuntimeClient(runtime_transport),
                addressing=AddressingClient(identity_transport),
            )
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

    def identity_client(self) -> IdentityClient:
        """Open the identity and addressing facade."""

        self._require_open()
        from . import _cabi

        transport = _cabi.open_cabi_identity_transport(
            control_path=self.resolved_control_path(),
            library_path=self.library_path,
        )
        return self._track(IdentityClient(transport))

    def addressing_client(self) -> AddressingClient:
        """Open the Axon-delegated URA and DescriptorRef helper facade."""

        self._require_open()
        from . import _cabi

        transport = _cabi.open_cabi_identity_transport(
            control_path=self.resolved_control_path(),
            library_path=self.library_path,
        )
        return self._track(AddressingClient(transport))

    def directory_client(self) -> DirectoryClient:
        """Open the Directory profile facade."""

        from . import _cabi

        return self._track(
            DirectoryClient(
                self._profile_transport(_cabi.open_cabi_directory_transport)
            )
        )

    def receipt_client(self) -> ReceiptClient:
        """Open the Receipt profile facade."""

        from . import _cabi

        return self._track(
            ReceiptClient(self._profile_transport(_cabi.open_cabi_receipt_transport))
        )

    def publication_client(self) -> PublicationClient:
        """Open the Publication profile facade."""

        from . import _cabi

        control_path = self.resolved_control_path()
        carrier = self._profile_transport(_cabi.open_cabi_publication_transport)
        runtime_transport = _cabi.open_cabi_runtime_transport(
            control_path=control_path,
            library_path=self.library_path,
        )
        return self._track(
            PublicationClient(
                RuntimePublicationTransport(
                    carrier=carrier,
                    runtime=RuntimeClient(runtime_transport),
                )
            )
        )

    def host_binding_client(self) -> HostBindingClient:
        """Open the Host Binding profile facade."""

        from . import _cabi

        return self._track(
            HostBindingClient(
                self._profile_transport(_cabi.open_cabi_host_binding_transport)
            )
        )

    def mission_client(self) -> MissionClient:
        """Open the Mission profile facade."""

        from . import _cabi

        control_path = self.resolved_control_path()
        carrier = self._profile_transport(_cabi.open_cabi_mission_transport)
        runtime_transport = _cabi.open_cabi_runtime_transport(
            control_path=control_path,
            library_path=self.library_path,
        )
        return self._track(
            MissionClient(
                RuntimeMissionTransport(
                    carrier=carrier,
                    runtime=RuntimeClient(runtime_transport),
                )
            )
        )

    def admin_client(self) -> AdminClient:
        """Open the Admin + Gateway profile facade."""

        from . import _cabi

        return self._track(
            AdminClient(self._profile_transport(_cabi.open_cabi_admin_transport))
        )

    def event_client(self) -> EventClient:
        """Open the Events profile facade."""

        from . import _cabi

        return self._track(
            EventClient(self._profile_transport(_cabi.open_cabi_events_transport))
        )

    def surface_client(self) -> SurfaceClient:
        """Open the Surface profile facade."""

        from . import _cabi

        control_path = self.resolved_control_path()
        carrier = self._profile_transport(_cabi.open_cabi_surface_transport)
        runtime_transport = _cabi.open_cabi_runtime_transport(
            control_path=control_path,
            library_path=self.library_path,
        )
        return self._track(
            SurfaceClient(
                RuntimeSurfaceTransport(
                    carrier=carrier,
                    runtime=RuntimeClient(runtime_transport),
                )
            )
        )

    def compatibility_client(self) -> CompatibilityClient:
        """Open the Compatibility profile facade."""

        from . import _cabi

        control_path = self.resolved_control_path()
        carrier = self._profile_transport(_cabi.open_cabi_compatibility_transport)
        runtime_transport = _cabi.open_cabi_runtime_transport(
            control_path=control_path,
            library_path=self.library_path,
        )
        return self._track(
            CompatibilityClient(
                RuntimeCompatibilityTransport(
                    carrier=carrier,
                    runtime=RuntimeClient(runtime_transport),
                )
            )
        )

    def wrapper_client(self) -> WrapperClient:
        """Open the Convenience Wrapper profile facade."""

        from . import _cabi

        control_path = self.resolved_control_path()
        carrier = self._profile_transport(_cabi.open_cabi_wrapper_transport)
        runtime_transport = _cabi.open_cabi_runtime_transport(
            control_path=control_path,
            library_path=self.library_path,
        )
        return self._track(
            WrapperClient(
                RuntimeWrapperTransport(
                    carrier=carrier,
                    runtime=RuntimeClient(runtime_transport),
                )
            )
        )

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
