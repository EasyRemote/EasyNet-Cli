"""Typed Python SDK errors."""

from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum
from typing import Optional


class ErrorCode(StrEnum):
    """Stable Python SDK error classification."""

    INVALID_ARGUMENT = "InvalidArgument"
    DAEMON_DOWN = "DaemonDown"
    VERSION_INCOMPATIBLE = "VersionIncompatible"
    TRANSPORT = "Transport"


class RetryHint(StrEnum):
    """Retry classification for SDK errors."""

    NEVER = "never"
    SAFE = "safe"


@dataclass(frozen=True)
class SDKError(Exception):
    """Typed error boundary used by Python SDK callers."""

    code: ErrorCode
    stage: str
    retry: RetryHint
    message: str
    cause: Optional[BaseException] = None

    def __post_init__(self) -> None:
        Exception.__init__(self, self.message)

    def __str__(self) -> str:
        return f"{self.code}: {self.message}" if self.message else str(self.code)


def is_code(error: BaseException, code: ErrorCode) -> bool:
    """Return whether *error* is an SDKError with the requested code."""

    return isinstance(error, SDKError) and error.code == code
