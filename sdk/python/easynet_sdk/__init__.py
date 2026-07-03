"""Python facade for the EasyNet-Cli Daemon SDK.

The public package exposes SDK DTOs and typed errors. It intentionally does not
expose ctypes, raw C ABI handles, Axon protobufs, or daemon-internal modules.
"""

from .client import Client, DiscoveryTransport, FeatureSet, Version
from .errors import ErrorCode, RetryHint, RuntimeError, SDKError, is_code
from .health import HealthClient, HealthTransport, RuntimeHealth

__all__ = [
    "Client",
    "DiscoveryTransport",
    "ErrorCode",
    "FeatureSet",
    "HealthClient",
    "HealthTransport",
    "RetryHint",
    "RuntimeHealth",
    "RuntimeError",
    "SDKError",
    "Version",
    "is_code",
]
