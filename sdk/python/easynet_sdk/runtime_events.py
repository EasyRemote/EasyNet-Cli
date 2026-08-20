"""Runtime-event facade over the provider-neutral lifecycle core."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Mapping, Protocol

from .core.runtime_events import (
    RuntimeEventStreamState,
    validate_runtime_event_page_state,
)
from .errors import ErrorCode, RetryHint, SDKError
from .runtime import InvocationHandle, RuntimeClient

__all__ = [
    "DEFAULT_RUNTIME_EVENT_PAGE_LIMIT",
    "MAX_RUNTIME_EVENT_PAGE_LIMIT",
    "RuntimeEvent",
    "RuntimeEventClient",
    "RuntimeEventCursor",
    "RuntimeEventPage",
    "RuntimeEventProvider",
    "RuntimeEventReadRequest",
    "RuntimeEventStreamState",
    "RuntimeHandleEventProvider",
]

DEFAULT_RUNTIME_EVENT_PAGE_LIMIT = 50
MAX_RUNTIME_EVENT_PAGE_LIMIT = 500


@dataclass(frozen=True)
class RuntimeEventCursor:
    sequence: int = 0


@dataclass(frozen=True)
class RuntimeEvent:
    sequence: int
    kind: str
    state: str
    terminal: bool
    reason: str = ""
    result: Mapping[str, object] | None = None


@dataclass(frozen=True)
class RuntimeEventReadRequest:
    handle: InvocationHandle
    cursor: RuntimeEventCursor | None = None
    limit: int = 0


@dataclass(frozen=True)
class RuntimeEventPage:
    events: tuple[RuntimeEvent, ...]
    cursor: RuntimeEventCursor
    state: RuntimeEventStreamState
    terminal: bool
    limit: int

    def __post_init__(self) -> None:
        try:
            validate_runtime_event_page_state(self.state, terminal=self.terminal)
        except ValueError as error:
            raise _invalid_events(str(error)) from error


class RuntimeEventProvider(Protocol):
    def read_events(self, request: RuntimeEventReadRequest) -> RuntimeEventPage: ...


class RuntimeEventClient:
    def __init__(self, provider: RuntimeEventProvider) -> None:
        if provider is None:
            raise _invalid_events("runtime event provider is required")
        self._provider = provider

    def read(self, request: RuntimeEventReadRequest) -> RuntimeEventPage:
        return self._provider.read_events(request)


class RuntimeHandleEventProvider:
    def __init__(self, runtime: RuntimeClient) -> None:
        if runtime is None:
            raise _invalid_events("runtime client is required")
        self._runtime = runtime

    def read_events(self, request: RuntimeEventReadRequest) -> RuntimeEventPage:
        try:
            request.handle.control_capability()
        except SDKError as exc:
            raise _invalid_events(exc.message) from exc
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
            events.append(
                RuntimeEvent(
                    sequence=event.sequence,
                    kind=event.kind,
                    state=event.state,
                    terminal=event.terminal,
                    reason=event.reason or "",
                    result=event.result,
                )
            )
            cursor = RuntimeEventCursor(event.sequence)
        state = RuntimeEventStreamState.LIVE
        if snapshot.terminal:
            state = RuntimeEventStreamState.TERMINAL
        return RuntimeEventPage(
            events=tuple(events),
            cursor=cursor,
            state=state,
            terminal=snapshot.terminal,
            limit=limit,
        )


def _normalize_limit(limit: int) -> int:
    if not isinstance(limit, int) or isinstance(limit, bool) or limit < 0:
        raise _invalid_events("runtime event page limit must be non-negative")
    if limit == 0:
        return DEFAULT_RUNTIME_EVENT_PAGE_LIMIT
    if limit > MAX_RUNTIME_EVENT_PAGE_LIMIT:
        raise _invalid_events("runtime event page limit exceeds maximum")
    return limit


def _invalid_events(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime_events",
        retry=RetryHint.NEVER,
        message=message,
    )
