"""Python facade for the EasyNet-Cli Daemon SDK.

The public package exposes SDK DTOs and typed errors. It intentionally does not
expose ctypes, raw C ABI handles, Axon protobufs, or daemon-internal modules.
"""

from .client import Client, DiscoveryTransport, FeatureSet, Version
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

__all__ = [
    "Client",
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
    "PrepareOptions",
    "PreparedInvocation",
    "RetryHint",
    "RuntimeClient",
    "RuntimeHealth",
    "RuntimeError",
    "RuntimeTransport",
    "SDKError",
    "SignedInvocation",
    "SignerPolicy",
    "SigningMaterial",
    "Version",
    "is_code",
]
