"""Product-neutral Runtime Events facade."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import StrEnum
from typing import Mapping, Protocol

from .errors import ErrorCode, RetryHint, SDKError
from .runtime import InvocationHandle, RuntimeClient
from .runtime_ability import RuntimeAbilityClient, RuntimeCallContext
from .invocation import InvocationDraft

DEFAULT_RUNTIME_EVENT_PAGE_LIMIT = 50
MAX_RUNTIME_EVENT_PAGE_LIMIT = 500


class RuntimeEventStreamState(StrEnum):
    """Runtime event stream page state."""

    LIVE = "Live"
    TERMINAL = "Terminal"
    FAILED = "Failed"


class RuntimeEventStreamKind(StrEnum):
    """Daemon runtime event stream kind."""

    DIRECTORY = "directory"
    DEVICE = "device"
    SESSION = "session"
    INVOCATION = "invocation"


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
class RuntimeEventSubscriptionCursor:
    """Typed cursor used when resuming daemon event subscriptions."""

    stream: str = ""
    sequence: int = 0
    token: str = ""

    def resume_token(self) -> str:
        if self.token.strip():
            return self.token.strip()
        if not self.stream.strip():
            return ""
        return f"{self.stream.strip()}:{self.sequence}"


@dataclass(frozen=True)
class RuntimeEventSubscriptionRequest:
    """Build a daemon runtime event subscription InvocationDraft."""

    call: RuntimeCallContext
    stream: RuntimeEventStreamKind
    realm: str = ""
    owner_ura: str = ""
    device_ura: str = ""
    agent_ura: str = ""
    session_id: str = ""
    invocation_id: str = ""
    resume_cursor: RuntimeEventSubscriptionCursor | None = None
    heartbeat_interval_ms: int = 0


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


class RuntimeEventSubscriptionProvider(Protocol):
    """Provider for event subscription draft construction."""

    def build_subscription(
        self, request: RuntimeEventSubscriptionRequest
    ) -> InvocationDraft: ...


class RuntimeEventClient:
    """Product-neutral Runtime Events facade."""

    def __init__(self, provider: RuntimeEventProvider) -> None:
        if provider is None:
            raise _invalid_events("runtime event provider is required")
        self._provider = provider

    def read(self, request: RuntimeEventReadRequest) -> RuntimeEventPage:
        return self._provider.read_events(request)


class RuntimeEventSubscriptionClient:
    """Product-neutral Runtime Events subscription facade."""

    def __init__(self, provider: RuntimeEventSubscriptionProvider) -> None:
        if provider is None:
            raise _invalid_events("runtime event subscription provider is required")
        self._provider = provider

    def build(self, request: RuntimeEventSubscriptionRequest) -> InvocationDraft:
        return self._provider.build_subscription(request)


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


class RuntimeAbilityEventSubscriptionProvider:
    """Provider-backed subscription builder over RuntimeAbilityClient."""

    def __init__(self, ability: RuntimeAbilityClient) -> None:
        if ability is None:
            raise _invalid_events("runtime ability client is required")
        self._ability = ability

    def build_subscription(
        self, request: RuntimeEventSubscriptionRequest
    ) -> InvocationDraft:
        if not isinstance(request, RuntimeEventSubscriptionRequest):
            raise _invalid_events("RuntimeEventSubscriptionRequest is required")
        ability = runtime_event_subscription_ability(request.stream)
        args: dict[str, object] = {}
        if request.stream is not RuntimeEventStreamKind.SESSION:
            args["stream"] = request.stream.value
            args["daemon_ability"] = ability
        _put_text(args, "realm", request.realm)
        _put_text(args, "owner_ura", request.owner_ura)
        _put_text(args, "device_ura", request.device_ura)
        _put_text(args, "agent_ura", request.agent_ura)
        _put_text(args, "session_id", request.session_id)
        _put_text(args, "invocation_id", request.invocation_id)
        if request.heartbeat_interval_ms > 0:
            args["heartbeat_interval_ms"] = request.heartbeat_interval_ms
        if request.resume_cursor is not None:
            _validate_resume_cursor(request.stream, request.resume_cursor)
            if request.stream is RuntimeEventStreamKind.SESSION:
                args["since_seq"] = request.resume_cursor.sequence
            else:
                token = request.resume_cursor.resume_token()
                if token:
                    args["resume_cursor"] = token
        metadata = dict(request.call.metadata)
        metadata["sdk_profile"] = "runtime_events"
        metadata["system_ability"] = ability
        call = RuntimeCallContext(
            caller_ura=request.call.caller_ura,
            callee_ura=request.call.callee_ura,
            subject_ura=request.call.subject_ura,
            descriptor_version=request.call.descriptor_version,
            nonce_base64=request.call.nonce_base64,
            causal_context=request.call.causal_context,
            metadata=metadata,
        )
        return self._ability.build(call, ability, args)


def runtime_event_subscription_ability(stream: RuntimeEventStreamKind) -> str:
    """Return the daemon system ability that serves a runtime event stream."""

    if stream is RuntimeEventStreamKind.DIRECTORY:
        return "federation.subscribe_directory_v2"
    if stream is RuntimeEventStreamKind.DEVICE:
        return "events.device.subscribe"
    if stream is RuntimeEventStreamKind.SESSION:
        return "session.attach"
    if stream is RuntimeEventStreamKind.INVOCATION:
        return "events.invocation.subscribe"
    raise _invalid_events(f"unsupported runtime event stream {stream!r}")


def _normalize_limit(limit: int) -> int:
    if limit == 0:
        return DEFAULT_RUNTIME_EVENT_PAGE_LIMIT
    if limit < 0 or limit > MAX_RUNTIME_EVENT_PAGE_LIMIT:
        raise _invalid_events("runtime event page limit exceeds maximum")
    return limit


def _validate_resume_cursor(
    stream: RuntimeEventStreamKind, cursor: RuntimeEventSubscriptionCursor
) -> None:
    cursor_stream = cursor.stream.strip()
    if not cursor_stream:
        raise _invalid_events("runtime event resume cursor stream is required")
    if cursor_stream != stream.value:
        raise _invalid_events(
            "runtime event resume cursor stream does not match subscription stream"
        )
    if cursor.sequence < 0:
        raise _invalid_events("runtime event resume cursor sequence must be non-negative")
    if cursor.token and cursor.token.strip() != cursor.token:
        raise _invalid_events("runtime event resume cursor token must be canonical")


def _put_text(values: dict[str, object], key: str, value: str) -> None:
    if value.strip():
        values[key] = value.strip()


def _invalid_events(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime_events",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )
