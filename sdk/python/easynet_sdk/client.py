"""Runtime Core discovery facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Mapping, Protocol, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError, retryable_for_hint


@runtime_checkable
class DiscoveryTransport(Protocol):
    """Narrow transport interface for Runtime Core feature discovery."""

    def feature_discovery(self) -> bytes:
        """Return raw feature discovery JSON bytes from a daemon SDK boundary."""

    def close(self) -> None:
        """Release discovery transport resources."""


@dataclass(frozen=True)
class Version:
    """Runtime Core version compatibility DTO."""

    abi_version: int
    sdk_version: str


@dataclass(frozen=True)
class FeatureSet:
    """Language-neutral SDK feature discovery DTO."""

    abi_version: int
    sdk_version: str
    profiles: Mapping[str, str] = field(default_factory=dict)
    symbols: Mapping[str, bool] = field(default_factory=dict)
    axon_pb: bool = False

    def version(self) -> Version:
        return Version(abi_version=self.abi_version, sdk_version=self.sdk_version)


class Client:
    """Python Runtime Core facade root."""

    def __init__(self, transport: DiscoveryTransport) -> None:
        if transport is None:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="sdk",
                retry=RetryHint.NEVER,
                message="discovery transport is required",
        )
        self._transport = transport
        self._closed = False

    def feature_discovery(self) -> FeatureSet:
        """Read and decode daemon SDK feature discovery."""

        transport = self._require_open()
        try:
            raw = transport.feature_discovery()
        except SDKError:
            raise
        except Exception as exc:
            raise SDKError(
                code=ErrorCode.ROUTE_UNAVAILABLE,
                stage="transport",
                retry=RetryHint.SAFE,
                retryable=retryable_for_hint(RetryHint.SAFE),
                message="feature discovery transport failed",
                cause=exc,
            ) from exc
        try:
            decoded = json.loads(raw)
        except Exception as exc:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="decode",
                retry=RetryHint.NEVER,
                message=f"decode feature discovery JSON: {exc}",
                cause=exc,
            ) from exc
        if not isinstance(decoded, dict):
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="decode",
                retry=RetryHint.NEVER,
                message="feature discovery JSON must be an object",
            )
        abi_version = decoded.get("abi_version")
        if (
            not isinstance(abi_version, int)
            or isinstance(abi_version, bool)
            or abi_version < 0
        ):
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="decode",
                retry=RetryHint.NEVER,
                message="abi_version must be a non-negative integer",
            )
        sdk_version = decoded.get("sdk_version", "")
        if not isinstance(sdk_version, str):
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="decode",
                retry=RetryHint.NEVER,
                message="sdk_version must be a string",
            )
        profiles = _string_map(decoded.get("profiles", {}), "profiles")
        symbols = _bool_map(decoded.get("symbols", {}), "symbols")
        axon_pb = decoded.get("axon_pb", False)
        if not isinstance(axon_pb, bool):
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="decode",
                retry=RetryHint.NEVER,
                message="axon_pb must be a boolean",
            )
        return FeatureSet(
            abi_version=abi_version,
            sdk_version=sdk_version,
            profiles=profiles,
            symbols=symbols,
            axon_pb=axon_pb,
        )

    def close(self) -> None:
        """Release the SDK discovery boundary without stopping the daemon."""

        if self._closed:
            return
        self._closed = True
        try:
            self._transport.close()
        except SDKError:
            raise
        except Exception as exc:
            raise SDKError(
                code=ErrorCode.ROUTE_UNAVAILABLE,
                stage="transport",
                retry=RetryHint.SAFE,
                retryable=retryable_for_hint(RetryHint.SAFE),
                message="client close transport failed",
                cause=exc,
            ) from exc

    def _require_open(self) -> DiscoveryTransport:
        if self._closed:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="sdk",
                retry=RetryHint.NEVER,
                retryable=retryable_for_hint(RetryHint.NEVER),
                message="client is closed",
            )
        return self._transport

    def require_abi(self, expected: int) -> FeatureSet:
        """Return feature discovery or raise VersionMismatch."""

        if expected <= 0:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="sdk",
                retry=RetryHint.NEVER,
                message="expected ABI version must be positive",
            )
        features = self.feature_discovery()
        if features.abi_version != expected:
            raise SDKError(
                code=ErrorCode.VERSION_MISMATCH,
                stage="sdk",
                retry=RetryHint.NEVER,
                message=(
                    f"daemon ABI version {features.abi_version} "
                    f"does not match expected {expected}"
                ),
            )
        return features


def _string_map(value: object, field_name: str) -> Mapping[str, str]:
    if not isinstance(value, dict):
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="decode",
            retry=RetryHint.NEVER,
            message=f"{field_name} must be an object",
        )
    result: dict[str, str] = {}
    for key, item in value.items():
        if not isinstance(key, str) or not isinstance(item, str):
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="decode",
                retry=RetryHint.NEVER,
                message=f"{field_name} must map strings to strings",
            )
        result[key] = item
    return result


def _bool_map(value: object, field_name: str) -> Mapping[str, bool]:
    if not isinstance(value, dict):
        raise SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="decode",
            retry=RetryHint.NEVER,
            message=f"{field_name} must be an object",
        )
    result: dict[str, bool] = {}
    for key, item in value.items():
        if not isinstance(key, str) or not isinstance(item, bool):
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="decode",
                retry=RetryHint.NEVER,
                message=f"{field_name} must map strings to booleans",
            )
        result[key] = item
    return result
