"""Shared SDK client lifecycle helpers."""

from __future__ import annotations

from typing import Protocol, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError


@runtime_checkable
class CloseTransport(Protocol):
    def close(self) -> None:
        ...


class ClientLifecycle:
    """Local open/closed state for SDK facade clients."""

    def __init__(self, profile: str) -> None:
        self._profile = profile
        self._closed = False

    def require_open(self) -> None:
        if self._closed:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="sdk",
                retry=RetryHint.NEVER,
                message=f"{self._profile} client is closed",
            )

    def close(self, transport: object) -> None:
        if self._closed:
            return
        self._closed = True
        if isinstance(transport, CloseTransport):
            try:
                transport.close()
            except SDKError:
                raise
            except Exception as exc:
                raise SDKError(
                    code=ErrorCode.ROUTE_UNAVAILABLE,
                    stage="transport",
                    retry=RetryHint.SAFE,
                    message=f"{self._profile} close transport failed",
                    cause=exc,
                ) from exc
