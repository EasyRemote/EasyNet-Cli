"""Python facade for the EasyNet-Cli Daemon SDK.

The public package exposes SDK DTOs and typed errors. It intentionally does not
expose ctypes, raw C ABI handles, Axon protobufs, or daemon-internal modules.
"""

from .client import Client, DiscoveryTransport, FeatureSet, Version
from .connection import (
    ConnectOptions,
    ConnectionState,
    RuntimeConnection,
    RuntimeConnector,
    RuntimeEndpoint,
)
from .errors import ErrorCode, RetryHint, RuntimeError, SDKError, is_code
from .health import HealthClient, HealthTransport, RuntimeHealth
from .invocation import InvocationBuilder, InvocationDraft, InvocationSignature
from .runtime import (
    InvocationHandle,
    InvocationHandleEvent,
    InvocationCancel,
    InvocationFailure,
    InvocationResult,
    PrepareOptions,
    RuntimeClient,
    RuntimeTransport,
)
from .signing import (
    PreparedInvocation,
    SignedInvocation,
    SignerPolicy,
    SigningMaterial,
)
from .stream import (
    MAX_STREAM_BUFFERED_EVENTS,
    StreamCancel,
    StreamEvent,
    StreamHandle,
    StreamState,
    StreamTransport,
)

__all__ = [
    "Client",
    "ConnectOptions",
    "ConnectionState",
    "DiscoveryTransport",
    "ErrorCode",
    "FeatureSet",
    "HealthClient",
    "HealthTransport",
    "InvocationBuilder",
    "InvocationDraft",
    "InvocationHandle",
    "InvocationHandleEvent",
    "InvocationCancel",
    "InvocationFailure",
    "InvocationResult",
    "InvocationSignature",
    "MAX_STREAM_BUFFERED_EVENTS",
    "PrepareOptions",
    "PreparedInvocation",
    "RetryHint",
    "RuntimeClient",
    "RuntimeConnection",
    "RuntimeConnector",
    "RuntimeEndpoint",
    "RuntimeHealth",
    "RuntimeError",
    "RuntimeTransport",
    "SDKError",
    "SignedInvocation",
    "SignerPolicy",
    "SigningMaterial",
    "StreamCancel",
    "StreamEvent",
    "StreamHandle",
    "StreamState",
    "StreamTransport",
    "Version",
    "is_code",
]
