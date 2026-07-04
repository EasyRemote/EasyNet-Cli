"""Process-level SDK environment root."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Protocol, TypeVar

from .client import Client, FeatureSet
from .connection import ConnectOptions
from .daemon import DaemonControl
from .errors import ErrorCode, RetryHint, SDKError
from .health import HealthClient
from .identity import IdentityClient
from .runtime import RuntimeClient


class _Closable(Protocol):
    def close(self) -> None:
        ...


_TClosable = TypeVar("_TClosable", bound=_Closable)


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

    def connect_local(
        self, options: ConnectOptions = ConnectOptions()
    ) -> RuntimeClient:
        """Discover, attach, open, and detach a local daemon runtime client."""

        client = self.daemon_control().connect_local(options)
        return self._track(client)

    def runtime_client(self) -> RuntimeClient:
        """Open a direct runtime client for the configured control path."""

        self._require_open()
        from . import _cabi

        transport = _cabi.open_cabi_runtime_transport(
            control_path=self.control_path,
            library_path=self.library_path,
        )
        return self._track(RuntimeClient(transport))

    def health_client(self) -> HealthClient:
        """Open a health facade for the configured control path."""

        self._require_open()
        from . import _cabi

        transport = _cabi.open_cabi_runtime_transport(
            control_path=self.control_path,
            library_path=self.library_path,
        )
        self._track(transport)
        return HealthClient(transport)

    def identity_client(self) -> IdentityClient:
        """Open the identity and addressing facade."""

        self._require_open()
        from . import _cabi

        transport = _cabi.open_cabi_identity_transport(
            control_path=self.control_path,
            library_path=self.library_path,
        )
        return self._track(IdentityClient(transport))

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
                        code=ErrorCode.TRANSPORT,
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
