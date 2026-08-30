"""Process-level SDK environment root."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from threading import Lock
from typing import Protocol, TypeVar

from .ability_invocation import AbilityInvocationClient
from .client import Client, FeatureSet
from .connection import (
    ConnectOptions,
    _ControlDiscoveryRuntimeConnector,
    _connect_options_or_default,
    RuntimeConnection,
)
from .providers.runtime.control import _ControlIpcClient, _default_control_path
from .runtime_lifecycle import RuntimeLifecycle
from .errors import ErrorCode, RetryHint, SDKError
from .health import HealthClient
from .axon_addressing import AddressingClient
from .runtime import RuntimeClient
from .runtime_ability import RuntimeAbilityClient
from .runtime_authority import LocalRuntimeAuthorityProvider
from .runtime_signer import LocalRuntimeSignerProvider
from .signing import Signer
from .runtime_environment import (
    RuntimeIdentityProjection,
    read_paired_runtime_identity_projection,
    read_runtime_control_discovery,
    read_runtime_identity_projection,
    runtime_credentials_path,
    runtime_state_root,
)
from .transport import RuntimeInvocationTransport


class _Closable(Protocol):
    def close(self) -> None:
        ...


class _ProfileTransportOpener(Protocol):
    def __call__(
        self, *, control_path: str, library_path: str | None
    ) -> _Closable: ...


_TClosable = TypeVar("_TClosable", bound=_Closable)


@dataclass
class NativeRuntimeHandle:
    """One SDK-owned native Runtime and Health provider lifecycle.

    Health borrows Runtime's transport. Addressing is an Axon-backed local
    provider and never depends on a product profile exported by generic ABI v7.
    """

    _runtime: RuntimeClient
    _health: HealthClient
    _addressing: AddressingClient
    _authority: LocalRuntimeAuthorityProvider | None = None
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

    def ability_client(self) -> RuntimeAbilityClient:
        """Return a borrowed generic runtime ability facade."""

        self._require_open()
        return RuntimeAbilityClient(self._runtime, self._addressing, self._authority)

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
        """Return SDK feature discovery for the configured runtime provider."""

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
            _cabi.RuntimeCABILibrary.load(self.library_path)
        )
        return self._track(Client(transport))

    def runtime_lifecycle(self) -> RuntimeLifecycle:
        """Open runtime host lifecycle control over the SDK default transport."""

        self._require_open()
        from . import _cabi

        transport = _cabi.open_cabi_runtime_lifecycle_transport(
            library_path=self.library_path,
        )
        self._track(transport)
        return RuntimeLifecycle(transport)

    def _control_ipc_client(self, *, timeout: float | None = None) -> _ControlIpcClient:
        """Open a direct boot/status control IPC client."""

        self._require_open()
        return self._track(
            _ControlIpcClient.connect(
                self.resolved_control_path(),
                timeout=timeout,
            )
        )

    def connect_local(
        self, options: ConnectOptions | None = None
    ) -> RuntimeClient:
        """Discover, attach, open, and detach a local runtime host client."""

        options = _connect_options_or_default(options)
        client = self.runtime_lifecycle().connect_local(self._connect_options(options))
        return self._track(client)

    def runtime_connection(
        self, options: ConnectOptions | None = None
    ) -> RuntimeConnection:
        """Open a stateful RuntimeConnection over the SDK default connector."""

        self._require_open()
        from . import _cabi

        options = self._connect_options(_connect_options_or_default(options))
        connector = _ControlDiscoveryRuntimeConnector(
            _cabi.open_cabi_runtime_connector(library_path=self.library_path),
            control_path=options.control_path,
        )
        connection = RuntimeConnection(connector)
        connection.connect(options)
        return self._track(connection)

    def runtime_connection_direct(
        self, options: ConnectOptions | None = None
    ) -> RuntimeConnection:
        """Open a direct Axon runtime connection with canonical Addressing."""

        self._require_open()
        from .providers.runtime.direct import DirectRuntimeConnector

        addressing = _canonical_addressing_client()
        connector = DirectRuntimeConnector(
            control_path=self.resolved_control_path(),
            identity=addressing,
            close_identity=True,
        )
        connection = RuntimeConnection(connector)
        connection.connect(self._connect_options(_connect_options_or_default(options)))
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
        self, options: ConnectOptions | None = None
    ) -> NativeRuntimeHandle:
        """Open one owned native Runtime and Health provider."""

        self._require_open()
        from . import _cabi

        resolved = self._connect_options(_connect_options_or_default(options))
        runtime_transport = _cabi.open_cabi_runtime_transport(
            control_path=resolved.control_path,
            library_path=self.library_path,
        )
        runtime = RuntimeClient(runtime_transport)
        health = HealthClient(runtime_transport, owns_transport=False)
        addressing = _canonical_addressing_client()
        return self._track(
            NativeRuntimeHandle(
                runtime,
                health,
                addressing,
                self.local_runtime_authority_provider(addressing),
            )
        )

    def runtime_client_direct(
        self, options: ConnectOptions | None = None
    ) -> RuntimeClient:
        """Open a direct Axon gRPC-over-UDS runtime client."""

        return self._track(self.runtime_connection_direct(options).runtime_client())

    def invocation_transport(self) -> RuntimeInvocationTransport:
        """Open the public JSON-friendly Runtime Invocation transport facade."""

        self._require_open()
        return self._track(
            RuntimeInvocationTransport.connect(
                control_path=self.resolved_control_path(),
                library_path=self.library_path,
            )
        )

    def invocation_transport_direct(
        self, options: ConnectOptions | None = None
    ) -> RuntimeInvocationTransport:
        """Open direct UDS dispatch with local C ABI prepare/signing support."""

        self._require_open()
        from . import _cabi
        from .providers.runtime.direct import DirectRuntimeConnector

        resolved = self._connect_options(_connect_options_or_default(options))
        addressing = _canonical_addressing_client()
        handle_transport = _cabi.open_cabi_runtime_transport(
            control_path=resolved.control_path,
            library_path=self.library_path,
        )
        connector = DirectRuntimeConnector(
            control_path=resolved.control_path,
            handle_transport=handle_transport,
            identity=addressing,
            close_identity=True,
            close_handle_transport=True,
        )
        connection = RuntimeConnection(connector)
        try:
            connection.connect(resolved)
        except BaseException:
            connector.close()
            raise
        return self._track(
            RuntimeInvocationTransport(connection.runtime_client(), connection)
        )

    def ability_invocation_client(self) -> AbilityInvocationClient:
        """Open the generic complete-Invocation facade."""

        self._require_open()
        runtime = self.runtime_client()
        addressing = _canonical_addressing_client()
        return self._track(
            AbilityInvocationClient(
                runtime,
                addressing,
                self.local_runtime_authority_provider(addressing),
            )
        )

    def local_runtime_authority_provider(
        self,
        addressing: AddressingClient,
    ) -> LocalRuntimeAuthorityProvider:
        """Create authority policy bound to this runtime's key service."""

        self._require_open()
        return LocalRuntimeAuthorityProvider(
            addressing,
            key_service_path=str(self.runtime_state_root() / "keyring.sock"),
        )

    def local_runtime_invocation_signer(self, caller_ura: str) -> Signer:
        """Resolve the daemon-custodied active signer for a local caller."""

        return self.local_runtime_signer_provider().resolve(caller_ura)

    def local_runtime_signer_provider(self) -> LocalRuntimeSignerProvider:
        """Create managed signer selection bound to this runtime's key service."""

        self._require_open()
        return LocalRuntimeSignerProvider(
            key_service_path=str(self.runtime_state_root() / "keyring.sock"),
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
        """Close SDK-owned resources without stopping runtime-host processes."""

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
        """Return the configured runtime control discovery path or SDK default."""

        self._require_open()
        return self.control_path or str(_default_control_path())

    def runtime_state_root(self) -> Path:
        """Return the SDK-owned local runtime state directory."""

        return runtime_state_root(self.resolved_control_path())

    def runtime_credentials_path(self) -> Path:
        """Return the paired runtime identity projection path."""

        return runtime_credentials_path(self.resolved_control_path())

    def runtime_identity_projection(
        self,
        credentials_path: str | Path = "",
    ) -> RuntimeIdentityProjection:
        """Read this environment's public runtime identity.

        An explicit path is a strict standalone projection document. Without
        one, the running daemon's control discovery is the identity authority;
        its secret-bearing credentials store is never decoded as a public DTO.
        """

        if credentials_path:
            return read_runtime_identity_projection(
                credentials_path,
                control_path=self.resolved_control_path(),
            )
        discovery = read_runtime_control_discovery(self.resolved_control_path())
        identity = discovery.runtime_host_identity
        if (
            identity is None
            or not identity.realm.strip()
            or not identity.runtime_instance_id.strip()
        ):
            raise SDKError(
                code=ErrorCode.CALLER_IDENTITY_UNAVAILABLE,
                stage="runtime_environment",
                retry=RetryHint.NEVER,
                retryable=False,
                message="runtime control discovery has no complete runtime host identity",
            )
        return RuntimeIdentityProjection(
            realm=identity.realm,
            runtime_instance_id=identity.runtime_instance_id,
        )

    def paired_runtime_identity_projection(
        self,
        credentials_path: str | Path = "",
    ) -> RuntimeIdentityProjection:
        """Return the paired principal bound to this attached runtime.

        Secret-bearing credential persistence remains SDK-owned. The returned
        value contains only public identity, display, and control-plane facts.
        """

        self._require_open()
        return read_paired_runtime_identity_projection(
            credentials_path or self.runtime_credentials_path(),
            control_path=self.resolved_control_path(),
        )

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

    def _profile_transport(self, opener: _ProfileTransportOpener) -> _Closable:
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
