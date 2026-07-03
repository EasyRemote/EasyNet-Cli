"""Python facade for the EasyNet-Cli Daemon SDK.

The public package exposes SDK DTOs and typed errors. It intentionally does not
expose ctypes, raw C ABI handles, Axon protobufs, or daemon-internal modules.
"""

from .client import Client, DiscoveryTransport, FeatureSet, Version
from .bidi import (
    MAX_BIDI_BUFFERED_FRAMES,
    BidiFrame,
    BidiOutcome,
    BidiSession,
    BidiState,
    BidiStreamDescriptor,
    BidiTransport,
)
from .connection import (
    ConnectOptions,
    ConnectionState,
    RuntimeConnection,
    RuntimeConnector,
    RuntimeEndpoint,
)
from .daemon import (
    AttachOptions,
    DaemonControl,
    DaemonHandle,
    DaemonLifecycleState,
    DaemonMode,
    DaemonStatus,
    DaemonTransport,
    DiscoverOptions,
    Endpoints,
    StartConfig,
    StopOptions,
    attach_daemon,
    discover_daemon,
    start_daemon,
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
    "AttachOptions",
    "DaemonControl",
    "DaemonHandle",
    "DaemonLifecycleState",
    "DaemonMode",
    "DaemonStatus",
    "DaemonTransport",
    "DiscoverOptions",
    "DiscoveryTransport",
    "Endpoints",
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
    "MAX_BIDI_BUFFERED_FRAMES",
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
    "StartConfig",
    "StopOptions",
    "BidiFrame",
    "BidiOutcome",
    "BidiSession",
    "BidiState",
    "BidiStreamDescriptor",
    "BidiTransport",
    "StreamCancel",
    "StreamEvent",
    "StreamHandle",
    "StreamState",
    "StreamTransport",
    "Version",
    "attach_daemon",
    "discover_daemon",
    "is_code",
    "start_daemon",
]
