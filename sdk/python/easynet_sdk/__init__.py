"""Python facade for the EasyNet-Cli Daemon SDK.

The public package exposes SDK DTOs and typed errors. It intentionally does not
expose ctypes, raw C ABI handles, Axon protobufs, or daemon-internal modules.
"""

from .client import Client, DiscoveryTransport, FeatureSet, Version
from .errors import ErrorCode, RetryHint, SDKError, is_code

__all__ = [
    "Client",
    "DiscoveryTransport",
    "ErrorCode",
    "FeatureSet",
    "RetryHint",
    "SDKError",
    "Version",
    "is_code",
]
