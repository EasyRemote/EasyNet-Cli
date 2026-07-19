"""EasyNet connection lowering into canonical Invocation transport objects."""

from __future__ import annotations

from typing import Any

from ... import _cabi
from ... import transport as canonical_transport
from ...connection import (
    ConnectOptions,
    ControlDiscoveryRuntimeConnector,
    RuntimeConnection,
)
from ...errors import ErrorCode, RetryHint, SDKError


def connect_invocation_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
    options: ConnectOptions = ConnectOptions(),
) -> canonical_transport.RuntimeInvocationTransport:
    """Connect through the explicit easynet-daemon C ABI provider."""

    resolved_control_path = options.control_path or control_path
    connection = RuntimeConnection(
        ControlDiscoveryRuntimeConnector(
            _cabi.open_cabi_runtime_connector(library_path=library_path),
            control_path=resolved_control_path,
        )
    )
    return canonical_transport._open_runtime_invocation_transport(
        connection,
        _resolved_options(options, resolved_control_path),
    )


def connect_direct_invocation_transport(
    *,
    control_path: str = "",
    library_path: str | None = None,
    options: ConnectOptions = ConnectOptions(),
    identity: Any | None = None,
) -> canonical_transport.RuntimeInvocationTransport:
    """Connect to easynet-daemon's direct Axon gRPC-over-UDS endpoint."""

    if identity is None:
        raise SDKError(
            code=ErrorCode.NOT_IMPLEMENTED,
            stage="sdk",
            retry=RetryHint.NEVER,
            retryable=False,
            message=(
                "direct runtime requires an explicit Addressing provider; "
                "generic C ABI v5 does not export identity grammar"
            ),
        )

    resolved_control_path = options.control_path or control_path
    _ = library_path
    from ...direct_runtime import DirectRuntimeConnector

    connection = RuntimeConnection(
        DirectRuntimeConnector(
            control_path=resolved_control_path,
            identity=identity,
            close_identity=False,
        )
    )
    return canonical_transport._open_runtime_invocation_transport(
        connection,
        _resolved_options(options, resolved_control_path),
    )


def _resolved_options(options: ConnectOptions, control_path: str) -> ConnectOptions:
    return ConnectOptions(
        endpoint=options.endpoint,
        control_path=control_path,
        dial_timeout_ms=options.dial_timeout_ms,
        invoke_timeout_ms=options.invoke_timeout_ms,
        max_message_bytes=options.max_message_bytes,
        reconnect=options.reconnect,
    )
