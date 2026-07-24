"""Canonical runtime Directory facade and generic event projection."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import StrEnum
from typing import Mapping, Protocol

from .errors import ErrorCode, RetryHint, SDKError
from .core.directory import DirectoryResolveKind
from .runtime_ability import RuntimeCallContext
from .stream import StreamHandle

__all__ = [
    "DEFAULT_DIRECTORY_PAGE_LIMIT",
    "MAX_DIRECTORY_PAGE_LIMIT",
    "DirectoryClient",
    "DirectoryCursor",
    "DirectoryEvent",
    "DirectoryEventEnvelope",
    "DirectoryListRequest",
    "DirectoryPage",
    "DirectoryProvider",
    "DirectoryRecord",
    "DirectoryResolution",
    "DirectoryResolveKind",
    "DirectoryResolveRequest",
    "DirectorySubscribeRequest",
    "DirectorySubscription",
    "DirectorySubscriptionState",
    "parse_directory_event",
]

DEFAULT_DIRECTORY_PAGE_LIMIT = 50
MAX_DIRECTORY_PAGE_LIMIT = 500
MAX_DIRECTORY_CURSOR_LENGTH = 4096


@dataclass(frozen=True)
class DirectoryResolveRequest:
    call: RuntimeCallContext
    query_ura: str
    realm_hint: str = ""
    ability_name: str = ""
    kind: DirectoryResolveKind = DirectoryResolveKind.ROUTE
    include_abilities: bool | None = None


@dataclass(frozen=True)
class DirectoryRecord:
    kind: str
    raw: Mapping[str, object]
    ura: str = ""
    owner_ura: str = ""
    ability_ura: str = ""
    route_ura: str = ""


@dataclass(frozen=True)
class DirectoryResolution:
    answer_kind: str
    records: tuple[DirectoryRecord, ...]
    canonical_ura: str = ""
    owner_ura: str = ""
    ability_ura: str = ""
    route_ura: str = ""
    next_hop: Mapping[str, object] = field(default_factory=dict)
    selected_route: Mapping[str, object] = field(default_factory=dict)
    route_candidates: tuple[Mapping[str, object], ...] = ()
    negative: Mapping[str, object] = field(default_factory=dict)
    next_cursor: str = ""
    release_profile: str = ""
    authority: Mapping[str, object] = field(default_factory=dict)
    cache_policy: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class DirectoryListRequest:
    call: RuntimeCallContext
    ura_prefix: str
    limit: int = 0
    cursor: str = ""
    include_abilities: bool | None = None


@dataclass(frozen=True)
class DirectoryPage:
    records: tuple[DirectoryRecord, ...]
    limit: int
    next_cursor: str = ""


@dataclass(frozen=True)
class DirectoryCursor:
    sequence: int
    token: str

    @classmethod
    def at(cls, sequence: int) -> "DirectoryCursor":
        if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 0:
            raise _invalid("Directory cursor sequence must be non-negative")
        return cls(sequence=sequence, token=f"directory:{sequence}")


@dataclass(frozen=True)
class DirectoryEvent:
    type: str
    raw: Mapping[str, object] = field(default_factory=dict)


class DirectorySubscriptionState(StrEnum):
    SNAPSHOTTING = "Snapshotting"
    LIVE = "Live"
    CLOSED = "Closed"
    FAILED = "Failed"


@dataclass(frozen=True)
class DirectorySubscribeRequest:
    call: RuntimeCallContext
    resume_cursor: DirectoryCursor | None = None


@dataclass(frozen=True)
class DirectoryEventEnvelope:
    cursor: DirectoryCursor
    event: DirectoryEvent | None = None
    terminal: bool = False


class DirectoryProvider(Protocol):
    def resolve(self, request: DirectoryResolveRequest) -> DirectoryResolution: ...
    def list(self, request: DirectoryListRequest) -> DirectoryPage: ...
    def subscribe(self, request: DirectorySubscribeRequest) -> "DirectorySubscription": ...


class DirectoryClient:
    def __init__(self, provider: DirectoryProvider) -> None:
        if provider is None:
            raise _invalid("Directory provider is required")
        self._provider = provider

    def resolve(self, request: DirectoryResolveRequest) -> DirectoryResolution:
        return self._provider.resolve(request)

    def list(self, request: DirectoryListRequest) -> DirectoryPage:
        return self._provider.list(request)

    def subscribe(self, request: DirectorySubscribeRequest) -> "DirectorySubscription":
        return self._provider.subscribe(request)


class DirectorySubscription:
    def __init__(self, handle: StreamHandle) -> None:
        if handle is None:
            raise _invalid("Directory stream handle is required")
        self._handle = handle
        self._state = DirectorySubscriptionState.SNAPSHOTTING
        self._cursor = DirectoryCursor.at(0)

    @property
    def state(self) -> DirectorySubscriptionState:
        return self._state

    @property
    def cursor(self) -> DirectoryCursor:
        return self._cursor

    def next(self, timeout: float | None = None) -> DirectoryEventEnvelope:
        if self._state in {
            DirectorySubscriptionState.CLOSED,
            DirectorySubscriptionState.FAILED,
        }:
            raise _invalid("Directory subscription is terminal")
        try:
            stream_event = self._handle.next(timeout)
            self._cursor = DirectoryCursor.at(stream_event.sequence)
            if stream_event.terminal:
                self._state = DirectorySubscriptionState.CLOSED
                return DirectoryEventEnvelope(cursor=self._cursor, terminal=True)
            event = parse_directory_event(stream_event.payload_json)
            self._transition(event)
            return DirectoryEventEnvelope(cursor=self._cursor, event=event)
        except Exception:
            self._state = DirectorySubscriptionState.FAILED
            raise

    def close(self) -> None:
        try:
            self._handle.close()
        except Exception:
            self._state = DirectorySubscriptionState.FAILED
            raise
        self._state = DirectorySubscriptionState.CLOSED

    def _transition(self, event: DirectoryEvent) -> None:
        if self._state == DirectorySubscriptionState.SNAPSHOTTING:
            if event.type != "snapshot":
                raise _invalid("Directory subscription requires snapshot as frame zero")
            self._state = DirectorySubscriptionState.LIVE
            return
        if self._state == DirectorySubscriptionState.LIVE:
            if event.type == "snapshot":
                raise _invalid("Directory subscription received a second snapshot")
            return
        raise _invalid("Directory subscription state is terminal")


def _project_resolution(output: Mapping[str, object]) -> DirectoryResolution:
    nested = output.get("answer")
    if nested is not None:
        if not isinstance(nested, Mapping):
            raise _invalid("Directory answer must be an object")
        output = nested
    answer_kind = _mapping_text(output, "answer_kind")
    if not answer_kind:
        raise _invalid("Directory answer_kind is required")
    records_raw = output.get("records", [])
    if not isinstance(records_raw, list):
        raise _invalid("Directory records must be a list")
    records = tuple(_project_record(item) for item in records_raw)
    return DirectoryResolution(
        answer_kind=answer_kind,
        canonical_ura=_mapping_text(output, "canonical_name"),
        owner_ura=_mapping_text(output, "owner_ura"),
        ability_ura=_mapping_text(output, "ability_ura"),
        route_ura=_mapping_text(output, "route_ura"),
        next_hop=_optional_mapping(output.get("next_hop"), "next_hop"),
        selected_route=_optional_mapping(output.get("selected_route"), "selected_route"),
        route_candidates=_optional_mapping_sequence(
            output.get("route_candidates"), "route_candidates"
        ),
        records=records,
        negative=_optional_mapping(output.get("negative"), "negative"),
        next_cursor=_mapping_text(output, "next_cursor"),
        release_profile=_mapping_text(output, "release_profile"),
        authority=_optional_mapping(output.get("authority"), "authority"),
        cache_policy=_optional_mapping(output.get("cache_policy"), "cache_policy"),
    )


def _project_record(value: object) -> DirectoryRecord:
    if not isinstance(value, Mapping):
        raise _invalid("Directory record must be an object")
    record = DirectoryRecord(
        kind=_mapping_text(value, "kind"),
        ura=_mapping_text(value, "ura"),
        owner_ura=_mapping_text(value, "owner_ura"),
        ability_ura=_mapping_text(value, "ability_ura"),
        route_ura=_mapping_text(value, "route_ura"),
        raw=dict(value),
    )
    if not record.kind:
        raise _invalid("Directory record kind is required")
    if not any((record.ura, record.owner_ura, record.ability_ura, record.route_ura)):
        raise _invalid("Directory record requires at least one canonical URA fact")
    return record


def parse_directory_event(raw: object) -> DirectoryEvent:
    value = _required_mapping(raw, "Directory event")
    event_type = _mapping_text(value, "type")
    if not event_type:
        raise _invalid("Directory event type is required")
    return DirectoryEvent(type=event_type, raw=value)


def _directory_limit(limit: int) -> int:
    if not isinstance(limit, int) or isinstance(limit, bool) or limit < 0:
        raise _invalid("Directory limit must be non-negative")
    if limit == 0:
        return DEFAULT_DIRECTORY_PAGE_LIMIT
    if limit > MAX_DIRECTORY_PAGE_LIMIT:
        raise _invalid("Directory limit exceeds the maximum page bound")
    return limit


def _directory_cursor(value: object) -> str:
    if not isinstance(value, str):
        raise _invalid("Directory cursor must be a string")
    cursor = value.strip()
    if len(cursor) > MAX_DIRECTORY_CURSOR_LENGTH:
        raise _invalid("Directory cursor exceeds the maximum bound")
    return cursor


def _negative_detail(resolution: DirectoryResolution) -> str:
    detail = resolution.negative.get("detail")
    if isinstance(detail, str) and detail.strip():
        return detail.strip()
    reason = resolution.negative.get("reason")
    if isinstance(reason, str) and reason.strip():
        return f"runtime Directory listing returned a negative answer: {reason.strip()}"
    return "runtime Directory listing returned a negative answer"


def _mapping_text(value: Mapping[str, object], key: str) -> str:
    raw = value.get(key)
    if isinstance(raw, str) and raw.strip():
        return raw.strip()
    return ""


def _required_mapping(value: object, name: str) -> Mapping[str, object]:
    if isinstance(value, bytes):
        try:
            import json

            decoded = json.loads(value)
        except (UnicodeDecodeError, ValueError) as error:
            raise _invalid(f"Directory JSON decode failed: {error}", error)
        if not isinstance(decoded, Mapping):
            raise _invalid(f"{name} must be an object")
        return dict(decoded)
    if isinstance(value, str):
        try:
            import json

            decoded = json.loads(value)
        except ValueError as error:
            raise _invalid(f"Directory JSON decode failed: {error}", error)
        if not isinstance(decoded, Mapping):
            raise _invalid(f"{name} must be an object")
        return dict(decoded)
    if not isinstance(value, Mapping):
        raise _invalid(f"{name} must be an object")
    return dict(value)


def _optional_mapping(value: object, name: str) -> Mapping[str, object]:
    if value is None:
        return {}
    if not isinstance(value, Mapping):
        raise _invalid(f"Directory {name} must be an object")
    return dict(value)


def _optional_mapping_sequence(
    value: object, name: str
) -> tuple[Mapping[str, object], ...]:
    if value is None:
        return ()
    if not isinstance(value, list):
        raise _invalid(f"Directory {name} must be a list")
    projected: list[Mapping[str, object]] = []
    for item in value:
        if not isinstance(item, Mapping):
            raise _invalid(f"Directory {name} item must be an object")
        projected.append(dict(item))
    return tuple(projected)


def _required_text(value: object, name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _invalid(f"{name} is required")
    return value.strip()


def _invalid(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="directory",
        retry=RetryHint.NEVER,
        message=message,
        cause=cause,
    )
