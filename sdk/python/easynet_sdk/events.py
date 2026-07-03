"""Events profile facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Callable, Mapping, Optional, Protocol, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError
from .invocation import InvocationDraft


_PROFILE = "events"
_DIRECTORY_STREAM = "directory"
MIN_EVENT_HEARTBEAT_INTERVAL_MS = 1000
MAX_EVENT_HEARTBEAT_INTERVAL_MS = 300000


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
class EventsDirectorySubscriptionRequest:
    base: EventsCarrierBase
    realm: str = ""
    owner_ura: str = ""
    device_ura: str = ""
    agent_ura: str = ""
    resume_cursor: Optional[EventCursor] = None
    heartbeat_interval_ms: int = 0

    def to_json_bytes(self) -> bytes:
        value = self.base.to_json_dict()
        for key, raw in (
            ("realm", self.realm),
            ("owner_ura", self.owner_ura),
            ("device_ura", self.device_ura),
            ("agent_ura", self.agent_ura),
        ):
            if raw:
                if raw.strip() != raw:
                    raise _invalid_events(f"{key} must not contain surrounding whitespace")
                value[key] = raw
        if self.resume_cursor is not None:
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


DirectoryEventQuery = EventsDirectorySubscriptionRequest


@dataclass(frozen=True)
class EventProjectionInput:
    cursor: EventCursor
    event: Mapping[str, object]
    event_id: str = ""
    resume_token: str = ""
    tenant_ref: object = None

    def to_json_bytes(self) -> bytes:
        _validate_cursor(self.cursor, require_token=False)
        if self.event is None:
            raise _invalid_events("directory event payload is required")
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

    @classmethod
    def from_json(cls, raw: bytes | str) -> "EventStream":
        decoded = _json_object(raw, "event stream")
        if decoded.get("stream") != _DIRECTORY_STREAM:
            raise _invalid_events("invalid event stream projection")
        return cls(
            stream=_required_string(decoded, "stream"),
            state=_required_string(decoded, "state"),
            stream_id=_optional_string(decoded.get("stream_id"), "stream_id") or "",
            resume_token=_optional_string(decoded.get("resume_token"), "resume_token") or "",
            metadata=_required_mapping(decoded, "metadata"),
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
        decoded = _json_object(raw, "event frame")
        if decoded.get("profile") != _PROFILE or decoded.get("stream") != _DIRECTORY_STREAM:
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
EventDropReport = EventFrame


@runtime_checkable
class EventTransport(Protocol):
    """Concrete Events operations supplied by the integration layer."""

    def build_directory_subscription_invocation(self, request_json: bytes) -> bytes:
        ...

    def subscribe_directory(self, request_json: bytes) -> bytes:
        ...

    def project_directory_event(self, event_json: bytes) -> bytes:
        ...

    def project_drop_report(self, drop_json: bytes) -> bytes:
        ...

    def project_terminal(self, terminal_json: bytes) -> bytes:
        ...


@dataclass(frozen=True)
class EventClient:
    """Events profile facade."""

    transport: EventTransport

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_events("events transport is required")

    def build_directory_subscription_invocation(
        self, request: EventsDirectorySubscriptionRequest
    ) -> InvocationDraft:
        return self._invocation(
            request.to_json_bytes(),
            self.transport.build_directory_subscription_invocation,
            "events directory subscription invocation failed",
        )

    def subscribe_directory(self, request: EventsDirectorySubscriptionRequest) -> EventStream:
        try:
            raw = self.transport.subscribe_directory(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("events subscribe directory failed", exc) from exc
        return EventStream.from_json(raw)

    def project_directory_event(self, input: EventProjectionInput) -> DirectoryEvent:
        return self._frame(
            input.to_json_bytes(),
            self.transport.project_directory_event,
            "events project directory event failed",
        )

    def project_drop_report(self, input: EventDropReportInput) -> EventDropReport:
        return self._frame(
            input.to_json_bytes(),
            self.transport.project_drop_report,
            "events project drop report failed",
        )

    def project_terminal(self, input: EventTerminalInput) -> EventFrame:
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


def _copy_optional_event_projection_fields(value: object, output: dict[str, object]) -> None:
    for key in ("reconnect_after_ms", "reason", "event_id", "resume_token", "tenant_ref"):
        raw = getattr(value, key)
        if raw is not None and raw != "":
            output[key] = raw


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
    if cursor.stream != _DIRECTORY_STREAM:
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
        cause=cause,
    )


def _transport_error(message: str, cause: BaseException) -> SDKError:
    return SDKError(
        code=ErrorCode.TRANSPORT,
        stage="transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=message,
        cause=cause,
    )
