"""Product-neutral Directory facade over Axon-owned wire projections."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import StrEnum
from typing import Mapping, Protocol

from easynet_axon.federation_directory import (
    DirectoryAgentSummary,
    DirectoryEntry,
    DirectoryEvent,
    DirectorySigningAuthority,
    parse_directory_event,
)

from .errors import ErrorCode, RetryHint, SDKError
from .runtime_ability import RuntimeAbilityClient, RuntimeCallContext
from .stream import StreamHandle

__all__ = [
    "DEFAULT_DIRECTORY_PAGE_LIMIT",
    "MAX_DIRECTORY_PAGE_LIMIT",
    "DirectoryAgentSummary",
    "DirectoryClient",
    "DirectoryCursor",
    "DirectoryEntry",
    "DirectoryEvent",
    "DirectoryEventEnvelope",
    "DirectoryListRequest",
    "DirectoryPage",
    "DirectoryProvider",
    "DirectoryRecord",
    "DirectoryResolution",
    "DirectoryResolveKind",
    "DirectoryResolveRequest",
    "DirectorySigningAuthority",
    "DirectorySubscribeRequest",
    "DirectorySubscription",
    "DirectorySubscriptionState",
    "RuntimeDirectoryProvider",
]

DEFAULT_DIRECTORY_PAGE_LIMIT = 50
MAX_DIRECTORY_PAGE_LIMIT = 500


class DirectoryResolveKind(StrEnum):
    ROUTE = "RESOLVE_TYPE_ROUTE"
    DIRECTORY_LISTING = "RESOLVE_TYPE_DIRECTORY_LISTING"
    CANONICAL_IDENTITY = "RESOLVE_TYPE_CANONICAL_IDENTITY"
    OWNER = "RESOLVE_TYPE_OWNER"


@dataclass(frozen=True)
class DirectoryResolveRequest:
    call: RuntimeCallContext
    query_ura: str
    realm_hint: str = ""
    ability_name: str = ""
    kind: DirectoryResolveKind = DirectoryResolveKind.ROUTE


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
    release_profile: str = ""
    authority: Mapping[str, object] = field(default_factory=dict)
    cache_policy: Mapping[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class DirectoryListRequest:
    call: RuntimeCallContext
    ura_prefix: str
    limit: int = 0
    cursor: str = ""


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


class RuntimeDirectoryProvider:
    """Directory provider composed over the canonical runtime ability kernel."""

    def __init__(self, ability: RuntimeAbilityClient) -> None:
        if ability is None:
            raise _invalid("runtime ability client is required")
        self._ability = ability

    def resolve(self, request: DirectoryResolveRequest) -> DirectoryResolution:
        query_ura = request.query_ura.strip()
        realm_hint = request.realm_hint.strip()
        if not query_ura and not realm_hint:
            raise _invalid("query_ura or realm_hint is required")
        arguments: dict[str, object] = {"qtype": request.kind.value}
        if query_ura:
            arguments["query_name"] = query_ura
        if realm_hint:
            arguments["realm_hint"] = realm_hint
        if request.ability_name.strip():
            arguments["ability_name"] = request.ability_name.strip()
        output = self._ability.invoke(request.call, "namespace.resolve", arguments)
        return _project_resolution(output)

    def list(self, request: DirectoryListRequest) -> DirectoryPage:
        if request.cursor.strip():
            raise _invalid("runtime Directory list cursor provider is not available")
        limit = _directory_limit(request.limit)
        resolution = self.resolve(
            DirectoryResolveRequest(
                call=request.call,
                query_ura=request.ura_prefix,
                kind=DirectoryResolveKind.DIRECTORY_LISTING,
            )
        )
        if len(resolution.records) > limit:
            raise _invalid(
                "runtime Directory listing exceeds the bounded page and has no stable cursor"
            )
        return DirectoryPage(records=resolution.records, limit=limit)

    def subscribe(self, request: DirectorySubscribeRequest) -> "DirectorySubscription":
        if request.resume_cursor is not None and request.resume_cursor.sequence != 0:
            raise _invalid("runtime Directory resume provider is not available")
        handle = self._ability.open_stream(
            request.call, "federation.subscribe_directory_v2", {}
        )
        return DirectorySubscription(handle)


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
    if isinstance(nested, Mapping):
        output = nested
    answer_kind = _mapping_text(output, "answer_kind")
    if not answer_kind and _mapping(output.get("negative")):
        answer_kind = "RESOLVE_ANSWER_KIND_NEGATIVE"
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
        next_hop=_mapping(output.get("next_hop")),
        selected_route=_mapping(output.get("selected_route")),
        route_candidates=_mapping_sequence(output.get("route_candidates")),
        records=records,
        negative=_mapping(output.get("negative")),
        release_profile=_mapping_text(output, "release_profile"),
        authority=_mapping(output.get("authority")),
        cache_policy=_mapping(output.get("cache_policy")),
    )


def _project_record(value: object) -> DirectoryRecord:
    if not isinstance(value, Mapping):
        raise _invalid("Directory record must be an object")
    return DirectoryRecord(
        kind=_mapping_text(value, "kind", "type"),
        ura=_mapping_text(value, "ura", "canonical_name"),
        owner_ura=_mapping_text(value, "owner_ura"),
        ability_ura=_mapping_text(value, "ability_ura"),
        route_ura=_mapping_text(value, "route_ura"),
        raw=dict(value),
    )


def _directory_limit(limit: int) -> int:
    if not isinstance(limit, int) or isinstance(limit, bool) or limit < 0:
        raise _invalid("Directory limit must be non-negative")
    if limit == 0:
        return DEFAULT_DIRECTORY_PAGE_LIMIT
    if limit > MAX_DIRECTORY_PAGE_LIMIT:
        raise _invalid("Directory limit exceeds the maximum page bound")
    return limit


def _mapping_text(value: Mapping[str, object], *keys: str) -> str:
    for key in keys:
        raw = value.get(key)
        if isinstance(raw, str) and raw.strip():
            return raw.strip()
    return ""


def _mapping(value: object) -> Mapping[str, object]:
    return dict(value) if isinstance(value, Mapping) else {}


def _mapping_sequence(value: object) -> tuple[Mapping[str, object], ...]:
    if not isinstance(value, list):
        return ()
    projected: list[Mapping[str, object]] = []
    for item in value:
        if not isinstance(item, Mapping):
            raise _invalid("Directory route candidate must be an object")
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
