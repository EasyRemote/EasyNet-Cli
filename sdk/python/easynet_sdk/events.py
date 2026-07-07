"""Events profile facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Callable, Mapping, Optional, Protocol, runtime_checkable

from ._lifecycle import ClientLifecycle
from .errors import ErrorCode, RetryHint, SDKError, profile_error_details
from .invocation import InvocationDraft
from .stream import StreamEvent, StreamHandle


_PROFILE = "events"
_DIRECTORY_STREAM = "directory"
_DEVICE_STREAM = "device"
_SESSION_STREAM = "session"
_INVOCATION_STREAM = "invocation"
_SUPPORTED_STREAMS = {
    _DIRECTORY_STREAM,
    _DEVICE_STREAM,
    _SESSION_STREAM,
    _INVOCATION_STREAM,
}
MIN_EVENT_HEARTBEAT_INTERVAL_MS = 1000
MAX_EVENT_HEARTBEAT_INTERVAL_MS = 300000
DEFAULT_EVENT_PAGE_SIZE = 50
MAX_EVENT_PAGE_SIZE = 500


@dataclass(frozen=True)
class EventsCarrierBase:
    """Complete carrier context shared by Events operations."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    metadata: Mapping[str, object] = field(default_factory=dict)

    def to_json_dict(self) -> dict[str, object]:
        _validate_base(self)
        value: dict[str, object] = {
            "caller_ura": self.caller_ura,
            "callee_ura": self.callee_ura,
            "subject_ura": self.subject_ura,
            "descriptor_version": self.descriptor_version,
            "nonce_base64": self.nonce_base64,
            "causal_context": dict(self.causal_context),
        }
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return value


@dataclass(frozen=True)
class EventCursor:
    stream: str
    sequence: int
    token: str = ""

    def resume_token(self) -> str:
        return self.token or f"{self.stream}:{self.sequence}"

    def to_json_dict(self, *, include_token: bool = False) -> dict[str, object]:
        _validate_cursor(self, require_token=False)
        value: dict[str, object] = {"stream": self.stream, "sequence": self.sequence}
        if include_token and self.token:
            value["token"] = self.token
        return value


@dataclass(frozen=True)
class EventFilter:
    """Typed Events profile filter carrier lowered into daemon ability args."""

    realm: str = ""
    owner_ura: str = ""
    device_ura: str = ""
    agent_ura: str = ""
    session_id: str = ""
    invocation_id: str = ""

    def normalized_with(self, top_level: Mapping[str, str]) -> "EventFilter":
        values = {
            "realm": self.realm,
            "owner_ura": self.owner_ura,
            "device_ura": self.device_ura,
            "agent_ura": self.agent_ura,
            "session_id": self.session_id,
            "invocation_id": self.invocation_id,
        }
        normalized: dict[str, str] = {}
        for key, raw in values.items():
            value = raw.strip()
            if value != raw:
                raise _invalid_events(f"{key} must not contain surrounding whitespace")
            top_raw = top_level.get(key) or ""
            top = top_raw.strip()
            if top != top_raw:
                raise _invalid_events(f"{key} must not contain surrounding whitespace")
            if top and value and top != value:
                raise _invalid_events(f"{key} conflicts with filter field")
            if key in {"realm", "session_id", "invocation_id"} and any(
                ch.isspace() for ch in value
            ):
                raise _invalid_events(f"{key} must not contain whitespace")
            normalized[key] = value or top
        return EventFilter(**normalized)

@dataclass(frozen=True)
class EventsSubscriptionRequest:
    base: EventsCarrierBase
    filter: Optional[EventFilter] = None
    realm: str = ""
    owner_ura: str = ""
    device_ura: str = ""
    agent_ura: str = ""
    resume_cursor: Optional[EventCursor] = None
    heartbeat_interval_ms: int = 0
    stream: str = ""
    session_id: str = ""
    session_ura: str = ""
    invocation_id: str = ""

    def to_json_bytes(self, expected_stream: str = _DIRECTORY_STREAM) -> bytes:
        if expected_stream not in _SUPPORTED_STREAMS:
            raise _invalid_events("unsupported event stream")
        stream = self.stream or expected_stream
        if stream != expected_stream:
            raise _invalid_events("event subscription stream mismatch")
        event_filter = _normalized_event_filter(
            self.filter,
            {
                "realm": self.realm,
                "owner_ura": self.owner_ura,
                "device_ura": self.device_ura,
                "agent_ura": self.agent_ura,
                "session_id": self.session_id,
                "invocation_id": self.invocation_id,
            },
        )
        if expected_stream == _SESSION_STREAM:
            if self.session_ura:
                raise _invalid_events(
                    "session_ura cannot be converted into daemon session_id"
                )
            if not event_filter.session_id:
                raise _invalid_events("session_id is required")
        if expected_stream == _INVOCATION_STREAM and not event_filter.invocation_id:
            raise _invalid_events("invocation_id is required")
        value = self.base.to_json_dict()
        value["stream"] = stream
        if self.filter is not None:
            value["filter"] = {
                key: raw
                for key, raw in {
                    "realm": event_filter.realm,
                    "owner_ura": event_filter.owner_ura,
                    "device_ura": event_filter.device_ura,
                    "agent_ura": event_filter.agent_ura,
                    "session_id": event_filter.session_id,
                    "invocation_id": event_filter.invocation_id,
                }.items()
                if raw
            }
        for key, raw in (
            ("realm", event_filter.realm),
            ("owner_ura", event_filter.owner_ura),
            ("device_ura", event_filter.device_ura),
            ("agent_ura", event_filter.agent_ura),
            ("session_id", event_filter.session_id),
            ("session_ura", self.session_ura),
            ("invocation_id", event_filter.invocation_id),
        ):
            if raw:
                if raw.strip() != raw:
                    raise _invalid_events(f"{key} must not contain surrounding whitespace")
                value[key] = raw
        if self.resume_cursor is not None:
            if self.resume_cursor.stream != expected_stream:
                raise _invalid_events("resume cursor stream mismatch")
            value["resume_cursor"] = self.resume_cursor.to_json_dict()
        if self.heartbeat_interval_ms:
            if not (
                MIN_EVENT_HEARTBEAT_INTERVAL_MS
                <= self.heartbeat_interval_ms
                <= MAX_EVENT_HEARTBEAT_INTERVAL_MS
            ):
                raise _invalid_events("heartbeat_interval_ms exceeds bounds")
            value["heartbeat_interval_ms"] = self.heartbeat_interval_ms
        return _json_bytes(value)


@dataclass(frozen=True)
class DirectoryEventQuery(EventsSubscriptionRequest):
    pass


@dataclass(frozen=True)
class DeviceEventQuery(EventsSubscriptionRequest):
    pass


@dataclass(frozen=True)
class SessionEventQuery(EventsSubscriptionRequest):
    pass


@dataclass(frozen=True)
class InvocationEventQuery(EventsSubscriptionRequest):
    pass


@dataclass(frozen=True)
class EventsDeviceEventListRequest:
    base: EventsCarrierBase
    filter: Optional[EventFilter] = None
    device_ura: str = ""
    limit: int = 0
    cursor: str = ""

    def to_json_bytes(self) -> bytes:
        event_filter = _normalized_event_filter(
            self.filter,
            {"device_ura": self.device_ura},
        )
        value = self.base.to_json_dict()
        if self.filter is not None and event_filter.device_ura:
            value["filter"] = {"device_ura": event_filter.device_ura}
        for key, raw in (
            ("device_ura", event_filter.device_ura),
            ("cursor", self.cursor),
        ):
            if raw:
                if raw.strip() != raw:
                    raise _invalid_events(f"{key} must not contain surrounding whitespace")
                value[key] = raw
        limit = self.limit or DEFAULT_EVENT_PAGE_SIZE
        if limit < 1 or limit > MAX_EVENT_PAGE_SIZE:
            raise _invalid_events("event page limit exceeds bounds")
        value["limit"] = limit
        return _json_bytes(value)


@dataclass(frozen=True)
class EventProjectionInput:
    cursor: EventCursor
    event: Mapping[str, object]
    event_id: str = ""
    resume_token: str = ""
    tenant_ref: object = None

    def to_json_bytes(self, expected_stream: str | None = None) -> bytes:
        _validate_cursor(self.cursor, require_token=False)
        if expected_stream is not None:
            _require_cursor_stream(self.cursor, expected_stream)
        if self.event is None:
            raise _invalid_events("event payload is required")
        value: dict[str, object] = {
            "cursor": self.cursor.to_json_dict(),
            "event": dict(self.event),
        }
        if self.event_id:
            value["event_id"] = self.event_id
        if self.resume_token:
            value["resume_token"] = self.resume_token
        if self.tenant_ref is not None:
            value["tenant_ref"] = self.tenant_ref
        return _json_bytes(value)


@dataclass(frozen=True)
class EventDropReportInput:
    cursor: EventCursor
    occurred_unix_ms: int
    dropped_count: int
    reconnect_after_ms: Optional[int] = None
    reason: str = ""
    event_id: str = ""
    resume_token: str = ""
    tenant_ref: object = None

    def to_json_bytes(self) -> bytes:
        _validate_cursor(self.cursor, require_token=False)
        _require_cursor_stream(self.cursor, _DIRECTORY_STREAM)
        if self.occurred_unix_ms < 0:
            raise _invalid_events("occurred_unix_ms must be non-negative")
        if self.dropped_count <= 0:
            raise _invalid_events("dropped_count must be greater than zero")
        _validate_reconnect_after_ms(self.reconnect_after_ms)
        value: dict[str, object] = {
            "cursor": self.cursor.to_json_dict(),
            "occurred_unix_ms": self.occurred_unix_ms,
            "dropped_count": self.dropped_count,
        }
        _copy_optional_event_projection_fields(self, value)
        return _json_bytes(value)


@dataclass(frozen=True)
class EventTerminalInput:
    cursor: EventCursor
    occurred_unix_ms: int
    reconnect_after_ms: Optional[int] = None
    reason: str = ""
    event_id: str = ""
    resume_token: str = ""
    tenant_ref: object = None

    def to_json_bytes(self) -> bytes:
        _validate_cursor(self.cursor, require_token=False)
        _require_cursor_stream(self.cursor, _DIRECTORY_STREAM)
        if self.occurred_unix_ms < 0:
            raise _invalid_events("occurred_unix_ms must be non-negative")
        _validate_reconnect_after_ms(self.reconnect_after_ms)
        value: dict[str, object] = {
            "cursor": self.cursor.to_json_dict(),
            "occurred_unix_ms": self.occurred_unix_ms,
        }
        _copy_optional_event_projection_fields(self, value)
        return _json_bytes(value)


@dataclass(frozen=True)
class EventStream:
    stream: str
    state: str
    metadata: Mapping[str, object]
    stream_id: str = ""
    resume_token: str = ""
    _runtime_stream: Optional[StreamHandle] = field(
        default=None, repr=False, compare=False
    )
    _live_projector: Optional[Callable[[EventProjectionInput], "EventFrame"]] = field(
        default=None, repr=False, compare=False
    )

    @classmethod
    def from_json(cls, raw: bytes | str) -> "EventStream":
        decoded = _json_object(raw, "event stream")
        if decoded.get("stream") not in _SUPPORTED_STREAMS:
            raise _invalid_events("invalid event stream projection")
        return cls(
            stream=_required_string(decoded, "stream"),
            state=_required_string(decoded, "state"),
            stream_id=_optional_string(decoded.get("stream_id"), "stream_id") or "",
            resume_token=_optional_string(decoded.get("resume_token"), "resume_token") or "",
            metadata=_required_mapping(decoded, "metadata"),
        )

    @classmethod
    def from_runtime_stream(
        cls,
        stream: str,
        runtime_stream: StreamHandle,
        *,
        resume_token: str = "",
        metadata: Mapping[str, object] | None = None,
        live_projector: Callable[[EventProjectionInput], "EventFrame"] | None = None,
    ) -> "EventStream":
        if stream not in _SUPPORTED_STREAMS:
            raise _invalid_events("invalid event stream projection")
        if runtime_stream is None:
            raise _invalid_events("runtime stream handle is required")
        state = getattr(runtime_stream.state, "value", str(runtime_stream.state))
        return cls(
            stream=stream,
            state=state,
            stream_id=runtime_stream.stream_id,
            resume_token=resume_token,
            metadata=dict(metadata or {"profile": _PROFILE}),
            _runtime_stream=runtime_stream,
            _live_projector=live_projector,
        )

    def next(self, timeout: float | None = None) -> "EventFrame":
        if self._runtime_stream is None:
            raise _invalid_events("event stream is not backed by a runtime stream handle")
        event = self._runtime_stream.next(timeout)
        state = getattr(self._runtime_stream.state, "value", str(self._runtime_stream.state))
        object.__setattr__(self, "state", state)
        return self._frame_from_stream_event(event)

    def close(self) -> None:
        if self._runtime_stream is None:
            return
        self._runtime_stream.close()
        state = getattr(self._runtime_stream.state, "value", str(self._runtime_stream.state))
        object.__setattr__(self, "state", state)

    def _frame_from_stream_event(self, event: StreamEvent) -> "EventFrame":
        if event.payload_json is None:
            raise _invalid_events("event stream frame payload_json is required")
        try:
            return EventFrame.from_mapping(_payload_mapping(event.payload_json))
        except SDKError as exc:
            if self.stream not in {
                _DIRECTORY_STREAM,
                _DEVICE_STREAM,
                _INVOCATION_STREAM,
            } or self._live_projector is None:
                raise exc
        return self._live_projector(
            EventProjectionInput(
                cursor=EventCursor(self.stream, event.sequence),
                event=_payload_mapping(event.payload_json),
            )
        )


@dataclass(frozen=True)
class EventFrame:
    profile: str
    stream: str
    kind: str
    event_id: str
    cursor: EventCursor
    resume_token: str
    occurred_unix_ms: int
    occurred_at: str
    subject_ref: object
    tenant_ref: object
    payload: object
    dropped_count: int
    reconnect_after_ms: Optional[int]
    terminal: bool
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "EventFrame":
        return cls.from_mapping(_json_object(raw, "event frame"))

    @classmethod
    def from_mapping(cls, decoded: Mapping[str, object]) -> "EventFrame":
        if decoded.get("profile") != _PROFILE or decoded.get("stream") not in _SUPPORTED_STREAMS:
            raise _invalid_events("invalid event frame projection")
        cursor = _cursor_from_json(decoded.get("cursor"), require_token=True)
        dropped_count = _required_non_negative_int(decoded, "dropped_count")
        kind = _required_string(decoded, "kind")
        terminal = _required_bool(decoded, "terminal")
        if "drop_report" in kind and dropped_count == 0:
            raise _invalid_events("dropped_count must be greater than zero")
        if "terminal" in kind and not terminal:
            raise _invalid_events("terminal event frame must be terminal")
        return cls(
            profile=_required_string(decoded, "profile"),
            stream=_required_string(decoded, "stream"),
            kind=kind,
            event_id=_required_string(decoded, "event_id"),
            cursor=cursor,
            resume_token=_required_string(decoded, "resume_token"),
            occurred_unix_ms=_required_non_negative_int(decoded, "occurred_unix_ms"),
            occurred_at=_required_string(decoded, "occurred_at"),
            subject_ref=decoded.get("subject_ref"),
            tenant_ref=decoded.get("tenant_ref"),
            payload=decoded.get("payload"),
            dropped_count=dropped_count,
            reconnect_after_ms=_optional_non_negative_int(
                decoded.get("reconnect_after_ms"), "reconnect_after_ms"
            ),
            terminal=terminal,
            metadata=_required_mapping(decoded, "metadata"),
        )


DirectoryEvent = EventFrame
DeviceEvent = EventFrame
SessionEvent = EventFrame
InvocationEvent = EventFrame
EventDropReport = EventFrame


@dataclass(frozen=True)
class DeviceEventPage:
    profile: str
    stream: str
    item_kind: str
    items: tuple[DeviceEvent, ...]
    next_cursor: Optional[str]
    has_more: bool
    limit: int
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "DeviceEventPage":
        decoded = _json_object(raw, "device event page")
        if decoded.get("profile") != _PROFILE or decoded.get("stream") != _DEVICE_STREAM:
            raise _invalid_events("invalid device event page projection")
        limit = _required_non_negative_int(decoded, "limit")
        if limit < 1 or limit > MAX_EVENT_PAGE_SIZE:
            raise _invalid_events("event page limit exceeds bounds")
        items_raw = decoded.get("items")
        if not isinstance(items_raw, list):
            raise _invalid_events("items must be a list")
        items: list[DeviceEvent] = []
        for item in items_raw:
            if not isinstance(item, dict):
                raise _invalid_events("device event page item must be an object")
            if item.get("stream") != _DEVICE_STREAM:
                raise _invalid_events("device event page item stream mismatch")
            items.append(EventFrame.from_mapping(item))
        return cls(
            profile=_required_string(decoded, "profile"),
            stream=_required_string(decoded, "stream"),
            item_kind=_required_string(decoded, "item_kind"),
            items=tuple(items),
            next_cursor=_optional_string(decoded.get("next_cursor"), "next_cursor"),
            has_more=_required_bool(decoded, "has_more"),
            limit=limit,
            metadata=_required_mapping(decoded, "metadata"),
        )


@runtime_checkable
class EventTransport(Protocol):
    """Concrete Events operations supplied by the integration layer."""

    def build_directory_subscription_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_device_subscription_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_session_subscription_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_invocation_subscription_invocation(self, request_json: bytes) -> bytes:
        ...

    def subscribe_directory(self, request_json: bytes) -> bytes | EventStream:
        ...

    def subscribe_devices(self, request_json: bytes) -> bytes | EventStream:
        ...

    def subscribe_sessions(self, request_json: bytes) -> bytes | EventStream:
        ...

    def subscribe_invocations(self, request_json: bytes) -> bytes | EventStream:
        ...

    def list_device_events(self, request_json: bytes) -> bytes:
        ...

    def project_directory_event(self, event_json: bytes) -> bytes:
        ...

    def project_live_event(self, event_json: bytes) -> bytes:
        ...

    def project_drop_report(self, drop_json: bytes) -> bytes:
        ...

    def project_terminal(self, terminal_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class EventClient:
    """Events profile facade."""

    transport: EventTransport
    _lifecycle: ClientLifecycle = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_events("events transport is required")
        object.__setattr__(self, "_lifecycle", ClientLifecycle("events"))

    def build_directory_subscription_invocation(
        self, request: DirectoryEventQuery
    ) -> InvocationDraft:
        self._require_open()
        return self._subscription_invocation(request, _DIRECTORY_STREAM)

    def build_device_subscription_invocation(
        self, request: DeviceEventQuery
    ) -> InvocationDraft:
        self._require_open()
        return self._subscription_invocation(request, _DEVICE_STREAM)

    def build_session_subscription_invocation(
        self, request: SessionEventQuery
    ) -> InvocationDraft:
        self._require_open()
        return self._subscription_invocation(request, _SESSION_STREAM)

    def build_invocation_subscription_invocation(
        self, request: InvocationEventQuery
    ) -> InvocationDraft:
        self._require_open()
        return self._subscription_invocation(request, _INVOCATION_STREAM)

    def subscribe_directory(self, request: DirectoryEventQuery) -> EventStream:
        self._require_open()
        return self._subscribe(request, _DIRECTORY_STREAM)

    def subscribe_devices(self, request: DeviceEventQuery) -> EventStream:
        self._require_open()
        return self._subscribe(request, _DEVICE_STREAM)

    def subscribe_sessions(self, request: SessionEventQuery) -> EventStream:
        self._require_open()
        return self._subscribe(request, _SESSION_STREAM)

    def subscribe_invocations(
        self, request: InvocationEventQuery
    ) -> EventStream:
        self._require_open()
        return self._subscribe(request, _INVOCATION_STREAM)

    def list_device_events(self, request: EventsDeviceEventListRequest) -> DeviceEventPage:
        self._require_open()
        try:
            raw = self.transport.list_device_events(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("events list device events failed", exc) from exc
        return DeviceEventPage.from_json(raw)

    def _subscription_invocation(
        self, request: EventsSubscriptionRequest, stream: str
    ) -> InvocationDraft:
        fn, label = self._subscription_invocation_transport(stream)
        return self._invocation(
            request.to_json_bytes(stream),
            fn,
            label,
        )

    def _subscribe(self, request: EventsSubscriptionRequest, stream: str) -> EventStream:
        fn, label = self._subscribe_transport(stream)
        try:
            raw = fn(request.to_json_bytes(stream))
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        if isinstance(raw, EventStream):
            return raw
        return EventStream.from_json(raw)

    def _subscription_invocation_transport(
        self, stream: str
    ) -> tuple[Callable[[bytes], bytes], str]:
        if stream == _DIRECTORY_STREAM:
            return (
                self.transport.build_directory_subscription_invocation,
                "events directory subscription invocation failed",
            )
        if stream == _DEVICE_STREAM:
            return (
                self.transport.build_device_subscription_invocation,
                "events device subscription invocation failed",
            )
        if stream == _SESSION_STREAM:
            return (
                self.transport.build_session_subscription_invocation,
                "events session subscription invocation failed",
            )
        if stream == _INVOCATION_STREAM:
            return (
                self.transport.build_invocation_subscription_invocation,
                "events invocation subscription invocation failed",
            )
        raise _invalid_events("unsupported event stream")

    def _subscribe_transport(self, stream: str) -> tuple[Callable[[bytes], bytes], str]:
        if stream == _DIRECTORY_STREAM:
            return self.transport.subscribe_directory, "events subscribe directory failed"
        if stream == _DEVICE_STREAM:
            return self.transport.subscribe_devices, "events subscribe devices failed"
        if stream == _SESSION_STREAM:
            return self.transport.subscribe_sessions, "events subscribe sessions failed"
        if stream == _INVOCATION_STREAM:
            return (
                self.transport.subscribe_invocations,
                "events subscribe invocations failed",
            )
        raise _invalid_events("unsupported event stream")

    def project_directory_event(self, input: EventProjectionInput) -> DirectoryEvent:
        self._require_open()
        return self._frame(
            input.to_json_bytes(_DIRECTORY_STREAM),
            self.transport.project_directory_event,
            "events project directory event failed",
        )

    def project_live_event(self, input: EventProjectionInput) -> EventFrame:
        self._require_open()
        return self._frame(
            input.to_json_bytes(),
            self.transport.project_live_event,
            "events project live event failed",
        )

    def project_drop_report(self, input: EventDropReportInput) -> EventDropReport:
        self._require_open()
        return self._frame(
            input.to_json_bytes(),
            self.transport.project_drop_report,
            "events project drop report failed",
        )

    def project_terminal(self, input: EventTerminalInput) -> EventFrame:
        self._require_open()
        return self._frame(
            input.to_json_bytes(),
            self.transport.project_terminal,
            "events project terminal failed",
        )

    def _invocation(
        self, request_json: bytes, fn: Callable[[bytes], bytes], label: str
    ) -> InvocationDraft:
        try:
            raw = fn(request_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        return InvocationDraft.from_json(raw)

    def _frame(
        self, request_json: bytes, fn: Callable[[bytes], bytes], label: str
    ) -> EventFrame:
        try:
            raw = fn(request_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        return EventFrame.from_json(raw)

    def close(self) -> None:
        self._lifecycle.close(self.transport)

    def _require_open(self) -> None:
        self._lifecycle.require_open()


def _copy_optional_event_projection_fields(value: object, output: dict[str, object]) -> None:
    for key in ("reconnect_after_ms", "reason", "event_id", "resume_token", "tenant_ref"):
        raw = getattr(value, key)
        if raw is not None and raw != "":
            output[key] = raw


def _normalized_event_filter(
    event_filter: Optional[EventFilter],
    top_level: Mapping[str, str],
) -> EventFilter:
    if event_filter is None:
        return EventFilter(**{key: value for key, value in top_level.items() if key in {
            "realm",
            "owner_ura",
            "device_ura",
            "agent_ura",
            "session_id",
            "invocation_id",
        }})
    return event_filter.normalized_with(top_level)


def _validate_base(base: EventsCarrierBase) -> None:
    if (
        not base.caller_ura
        or not base.callee_ura
        or not base.subject_ura
        or not base.descriptor_version
        or not base.nonce_base64
        or base.causal_context is None
    ):
        raise _invalid_events("complete events invocation carrier is required")


def _validate_cursor(cursor: EventCursor, *, require_token: bool) -> None:
    if cursor.stream not in _SUPPORTED_STREAMS:
        raise _invalid_events("unsupported event stream")
    if cursor.sequence < 0:
        raise _invalid_events("event cursor sequence must be non-negative")
    token = cursor.resume_token()
    if require_token and not cursor.token:
        raise _invalid_events("event cursor token is required")
    if any(ch.isspace() for ch in cursor.stream) or any(ch.isspace() for ch in token):
        raise _invalid_events("event cursor must not contain whitespace")
    if token != f"{cursor.stream}:{cursor.sequence}":
        raise _invalid_events("event cursor token must match stream sequence")


def _require_cursor_stream(cursor: EventCursor, expected: str) -> None:
    if cursor.stream != expected:
        raise _invalid_events("event cursor stream mismatch")


def _validate_reconnect_after_ms(value: Optional[int]) -> None:
    if value is None:
        return
    if value < 0 or value > MAX_EVENT_HEARTBEAT_INTERVAL_MS:
        raise _invalid_events("reconnect_after_ms exceeds bounds")


def _cursor_from_json(value: object, *, require_token: bool) -> EventCursor:
    if not isinstance(value, dict):
        raise _invalid_events("cursor must be an object")
    cursor = EventCursor(
        stream=_required_string(value, "stream"),
        sequence=_required_non_negative_int(value, "sequence"),
        token=_optional_string(value.get("token"), "token") or "",
    )
    _validate_cursor(cursor, require_token=require_token)
    return cursor


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _json_object(raw: bytes | str, label: str) -> dict[str, object]:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_events(f"decode {label} JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_events(f"{label} JSON must be an object")
    return decoded


def _payload_mapping(raw: object) -> Mapping[str, object]:
    if isinstance(raw, dict):
        return dict(raw)
    if isinstance(raw, bytes):
        raw = raw.decode("utf-8")
    if isinstance(raw, str):
        try:
            decoded = json.loads(raw)
        except Exception as exc:
            raise _invalid_events(f"decode event stream payload_json: {exc}", exc) from exc
        if isinstance(decoded, dict):
            return decoded
    raise _invalid_events("event stream frame payload_json must be an object")


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_events(f"{field_name} is required")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_events(f"{field_name} must be a string or null")
    return value


def _required_bool(decoded: Mapping[str, object], field_name: str) -> bool:
    value = decoded.get(field_name)
    if not isinstance(value, bool):
        raise _invalid_events(f"{field_name} must be a boolean")
    return value


def _required_non_negative_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_events(f"{field_name} must be a non-negative integer")
    return value


def _optional_non_negative_int(value: object, field_name: str) -> Optional[int]:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_events(f"{field_name} must be a non-negative integer or null")
    return value


def _required_mapping(decoded: Mapping[str, object], field_name: str) -> Mapping[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise _invalid_events(f"{field_name} must be an object")
    return dict(value)


def _invalid_events(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="events",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details=profile_error_details(_PROFILE),
        cause=cause,
    )


def _transport_error(message: str, cause: BaseException) -> SDKError:
    return SDKError(
        code=ErrorCode.ROUTE_UNAVAILABLE,
        stage="transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=message,
        details=profile_error_details(_PROFILE),
        cause=cause,
    )
