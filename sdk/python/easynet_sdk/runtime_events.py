"""Product-neutral Runtime Events facade."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import StrEnum
from typing import Mapping, Protocol

from .errors import ErrorCode, RetryHint, SDKError
from .runtime import InvocationHandle, RuntimeClient

DEFAULT_RUNTIME_EVENT_PAGE_LIMIT = 50
MAX_RUNTIME_EVENT_PAGE_LIMIT = 500


class RuntimeEventStreamState(StrEnum):
    """Runtime event stream page state."""

    LIVE = "Live"
    TERMINAL = "Terminal"
    FAILED = "Failed"


@dataclass(frozen=True)
class RuntimeEventCursor:
    """Typed runtime event cursor."""

    sequence: int = 0


@dataclass(frozen=True)
class RuntimeEvent:
    """One product-neutral runtime event projection."""

    sequence: int
    kind: str
    state: str
    terminal: bool
    reason: str = ""
    result: Mapping[str, object] | None = None


@dataclass(frozen=True)
class RuntimeEventReadRequest:
    """Runtime event read request."""

    handle: InvocationHandle
    cursor: RuntimeEventCursor | None = None
    limit: int = 0


@dataclass(frozen=True)
class RuntimeEventPage:
    """Bounded runtime event page."""

    events: tuple[RuntimeEvent, ...]
    cursor: RuntimeEventCursor
    state: RuntimeEventStreamState
    terminal: bool
    limit: int


class RuntimeEventProvider(Protocol):
    """Provider for bounded runtime event reads."""

    def read_events(self, request: RuntimeEventReadRequest) -> RuntimeEventPage: ...


class RuntimeEventClient:
    """Product-neutral Runtime Events facade."""

    def __init__(self, provider: RuntimeEventProvider) -> None:
        if provider is None:
            raise _invalid_events("runtime event provider is required")
        self._provider = provider

    def read(self, request: RuntimeEventReadRequest) -> RuntimeEventPage:
        return self._provider.read_events(request)


class RuntimeHandleEventProvider:
    """Runtime Events provider backed by RuntimeClient handle snapshots."""

    def __init__(self, runtime: RuntimeClient) -> None:
        if runtime is None:
            raise _invalid_events("runtime client is required")
        self._runtime = runtime

    def read_events(self, request: RuntimeEventReadRequest) -> RuntimeEventPage:
        if request.handle.handle_id <= 0:
            raise _invalid_events("handle_id is required")
        limit = _normalize_limit(request.limit)
        after = request.cursor.sequence if request.cursor is not None else 0
        snapshot = self._runtime.events(request.handle)
        events: list[RuntimeEvent] = []
        cursor = RuntimeEventCursor(after)
        for event in snapshot.events:
            if event.sequence <= after:
                continue
            if len(events) >= limit:
                break
            projected = RuntimeEvent(
                sequence=event.sequence,
                kind=event.kind,
                state=event.state,
                terminal=event.terminal,
                reason=event.reason or "",
                result=event.result,
            )
            events.append(projected)
            cursor = RuntimeEventCursor(event.sequence)
        state = (
            RuntimeEventStreamState.TERMINAL
            if snapshot.terminal
            else RuntimeEventStreamState.LIVE
        )
        return RuntimeEventPage(
            events=tuple(events),
            cursor=cursor,
            state=state,
            terminal=snapshot.terminal,
            limit=limit,
        )


def _normalize_limit(limit: int) -> int:
    if limit == 0:
        return DEFAULT_RUNTIME_EVENT_PAGE_LIMIT
    if limit < 0 or limit > MAX_RUNTIME_EVENT_PAGE_LIMIT:
        raise _invalid_events("runtime event page limit exceeds maximum")
    return limit


def _invalid_events(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime_events",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )
