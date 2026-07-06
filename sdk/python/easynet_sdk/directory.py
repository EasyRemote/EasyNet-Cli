"""Directory + Identity read-model facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field, replace
from typing import Callable, Mapping, Optional, Protocol, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError, profile_error_details
from ._lifecycle import ClientLifecycle
from .invocation import InvocationDraft
from .stream import StreamHandle


DEFAULT_DIRECTORY_PAGE_SIZE = 50
MAX_DIRECTORY_PAGE_SIZE = 500
MAX_DIRECTORY_SUBSCRIPTION_BUFFERED_EVENTS = 1024
_PROFILE = "directory_identity"
_READ_MODEL = "read_model"
_DIRECTORY_STREAM = "directory"
_DIRECTORY_SUBSCRIPTION_STATES = {
    "Opening",
    "CatchingUp",
    "Live",
    "Resuming",
    "Closed",
    "Failed",
}
_DIRECTORY_SUBSCRIPTION_TERMINAL_STATES = {"Closed", "Failed"}
_DIRECTORY_EVENT_PHASES = {"snapshot_start", "snapshot_complete", "live", "terminal"}


@dataclass(frozen=True)
class DirectoryQueryBase:
    """Complete carrier context for directory read-model requests."""

    caller_ura: str
    callee_ura: str
    subject_ura: str
    descriptor_version: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    limit: int = 0
    cursor: str = ""
    metadata: Mapping[str, object] = field(default_factory=dict)

    def with_default_limit(self) -> "DirectoryQueryBase":
        if self.limit == 0:
            return replace(self, limit=DEFAULT_DIRECTORY_PAGE_SIZE)
        return self

    def to_json_dict(self) -> dict[str, object]:
        value: dict[str, object] = {
            "caller_ura": self.caller_ura,
            "callee_ura": self.callee_ura,
            "subject_ura": self.subject_ura,
            "descriptor_version": self.descriptor_version,
            "nonce_base64": self.nonce_base64,
            "causal_context": dict(self.causal_context),
        }
        if self.limit:
            value["limit"] = self.limit
        if self.cursor:
            value["cursor"] = self.cursor
        if self.metadata:
            value["metadata"] = dict(self.metadata)
        return value


@dataclass(frozen=True)
class ResolveQuery:
    """Directory resolve query projection."""

    base: DirectoryQueryBase
    query_name: str = ""
    ability_name: str = ""
    qtype: str = ""
    realm_hint: str = ""

    def to_json_bytes(self) -> bytes:
        _validate_base(self.base, require_limit=False)
        if not self.query_name and not self.realm_hint:
            raise _invalid_directory("query_name or realm_hint is required")
        value = self.base.to_json_dict()
        if self.query_name:
            value["query_name"] = self.query_name
        if self.ability_name:
            value["ability_name"] = self.ability_name
        if self.qtype:
            value["qtype"] = self.qtype
        if self.realm_hint:
            value["realm_hint"] = self.realm_hint
        return _json_bytes(value)


@dataclass(frozen=True)
class DeviceQuery:
    base: DirectoryQueryBase


@dataclass(frozen=True)
class AgentQuery:
    base: DirectoryQueryBase


@dataclass(frozen=True)
class AbilityQuery:
    base: DirectoryQueryBase
    scope: str = ""
    owner_ura: str = ""
    ability_ura: str = ""

    def to_json_bytes(self) -> bytes:
        base = self.base.with_default_limit()
        _validate_base(base, require_limit=True)
        value = base.to_json_dict()
        if self.scope:
            value["scope"] = self.scope
        if self.owner_ura:
            value["owner_ura"] = self.owner_ura
        if self.ability_ura:
            value["ability_ura"] = self.ability_ura
        return _json_bytes(value)


@dataclass(frozen=True)
class DirectorySubscriptionCursor:
    stream: str
    sequence: int
    token: str = ""

    def resume_token(self) -> str:
        return self.token or f"{self.stream}:{self.sequence}"

    def to_json_dict(self) -> dict[str, object]:
        _validate_subscription_cursor(self)
        value: dict[str, object] = {
            "stream": self.stream,
            "sequence": self.sequence,
            "token": self.resume_token(),
        }
        return value


@dataclass(frozen=True)
class DirectorySubscriptionRequest:
    base: DirectoryQueryBase
    stream: str = ""
    realm: str = ""
    owner_ura: str = ""
    device_ura: str = ""
    agent_ura: str = ""
    ability_ura: str = ""
    item_kind: str = ""
    resume_cursor: Optional[DirectorySubscriptionCursor] = None
    heartbeat_interval_ms: int = 0

    def to_json_bytes(self) -> bytes:
        _validate_base(self.base, require_limit=False)
        stream = self.stream or _DIRECTORY_STREAM
        if stream != _DIRECTORY_STREAM:
            raise _invalid_directory("directory subscription stream mismatch")
        value = self.base.to_json_dict()
        value["stream"] = stream
        for key, raw in (
            ("realm", self.realm),
            ("owner_ura", self.owner_ura),
            ("device_ura", self.device_ura),
            ("agent_ura", self.agent_ura),
            ("ability_ura", self.ability_ura),
            ("item_kind", self.item_kind),
        ):
            if raw:
                if raw.strip() != raw:
                    raise _invalid_directory(f"{key} must not contain surrounding whitespace")
                value[key] = raw
        if self.resume_cursor is not None:
            value["resume_cursor"] = self.resume_cursor.to_json_dict()
        if self.heartbeat_interval_ms < 0:
            raise _invalid_directory("heartbeat_interval_ms must be non-negative")
        if self.heartbeat_interval_ms:
            value["heartbeat_interval_ms"] = self.heartbeat_interval_ms
        return _json_bytes(value)


@dataclass(frozen=True)
class DirectorySubscriptionEvent:
    profile: str
    stream: str
    kind: str
    event_id: str
    phase: str
    cursor: DirectorySubscriptionCursor
    resume_token: str
    terminal: bool
    metadata: Mapping[str, object]
    item_kind: str = ""
    item: Optional[Mapping[str, object]] = None

    @classmethod
    def from_mapping(cls, raw: Mapping[str, object]) -> "DirectorySubscriptionEvent":
        cursor = _subscription_cursor_from_json(raw.get("cursor"))
        event = cls(
            profile=_required_string(raw, "profile"),
            stream=_required_string(raw, "stream"),
            kind=_required_string(raw, "kind"),
            event_id=_required_string(raw, "event_id"),
            phase=_required_string(raw, "phase"),
            item_kind=_optional_string(raw.get("item_kind"), "item_kind") or "",
            item=_optional_mapping(raw.get("item"), "item"),
            cursor=cursor,
            resume_token=_optional_string(raw.get("resume_token"), "resume_token")
            or cursor.resume_token(),
            terminal=_required_bool(raw, "terminal"),
            metadata=_required_mapping(raw, "metadata"),
        )
        _validate_subscription_event(event)
        return event


@dataclass(frozen=True)
class DirectorySubscription:
    profile: str
    kind: str
    stream: str
    state: str
    cursor: DirectorySubscriptionCursor
    resume_token: str
    events: tuple[DirectorySubscriptionEvent, ...]
    drop_count: int
    metadata: Mapping[str, object]
    _runtime_stream: Optional[StreamHandle] = field(
        default=None, repr=False, compare=False
    )

    @classmethod
    def from_json(cls, raw: bytes | str) -> "DirectorySubscription":
        decoded = _json_object(raw, "directory subscription")
        cursor = _subscription_cursor_from_json(decoded.get("cursor"))
        events_raw = decoded.get("events", [])
        if not isinstance(events_raw, list):
            raise _invalid_directory("directory subscription events must be a list")
        if len(events_raw) > MAX_DIRECTORY_SUBSCRIPTION_BUFFERED_EVENTS:
            raise _invalid_directory("directory subscription buffered events exceeds bounds")
        events = tuple(DirectorySubscriptionEvent.from_mapping(_mapping_object(item, "event")) for item in events_raw)
        subscription = cls(
            profile=_required_string(decoded, "profile"),
            kind=_required_string(decoded, "kind"),
            stream=_required_string(decoded, "stream"),
            state=_required_string(decoded, "state"),
            cursor=cursor,
            resume_token=_optional_string(decoded.get("resume_token"), "resume_token")
            or cursor.resume_token(),
            events=events,
            drop_count=_required_non_negative_int(decoded, "drop_count"),
            metadata=_required_mapping(decoded, "metadata"),
        )
        _validate_subscription(subscription)
        return subscription

    @classmethod
    def from_runtime_stream(
        cls,
        runtime_stream: StreamHandle,
        *,
        cursor: Optional[DirectorySubscriptionCursor] = None,
        metadata: Mapping[str, object] | None = None,
    ) -> "DirectorySubscription":
        if runtime_stream is None:
            raise _invalid_directory("runtime stream handle is required")
        cursor = cursor or DirectorySubscriptionCursor(_DIRECTORY_STREAM, 0)
        state = getattr(runtime_stream.state, "value", str(runtime_stream.state))
        if state == "Open":
            state = "Live"
        subscription = cls(
            profile=_PROFILE,
            kind="directory_subscription",
            stream=_DIRECTORY_STREAM,
            state=state,
            cursor=cursor,
            resume_token=cursor.resume_token(),
            events=(),
            drop_count=0,
            metadata=dict(metadata or {"profile": _PROFILE}),
            _runtime_stream=runtime_stream,
        )
        _validate_subscription(subscription)
        return subscription

    def apply_event(self, event: DirectorySubscriptionEvent) -> DirectorySubscriptionEvent:
        """Project one daemon-emitted directory event into this subscription."""

        try:
            _validate_subscription_event(event)
            state = _DirectorySubscriptionStateMachine.apply_event(self, event)
            events = self.events + (event,)
            overflow = max(0, len(events) - MAX_DIRECTORY_SUBSCRIPTION_BUFFERED_EVENTS)
            if overflow:
                events = events[overflow:]
            metadata = dict(self.metadata)
            if event.phase == "snapshot_complete":
                metadata["snapshot_complete"] = True
            if overflow:
                metadata["buffer_overflow"] = True
            self._replace_projection(
                state=state,
                cursor=event.cursor,
                resume_token=event.cursor.resume_token(),
                events=events,
                drop_count=self.drop_count + overflow,
                metadata=metadata,
            )
        except SDKError:
            object.__setattr__(self, "state", "Failed")
            raise
        return event

    def apply_drop_report(
        self,
        cursor: DirectorySubscriptionCursor,
        dropped_count: int,
        *,
        metadata: Mapping[str, object] | None = None,
    ) -> None:
        """Project a daemon drop report without pretending the SDK owns fan-out."""

        _validate_subscription_cursor(cursor)
        if (
            not isinstance(dropped_count, int)
            or isinstance(dropped_count, bool)
            or dropped_count <= 0
        ):
            raise _invalid_directory(
                "directory subscription dropped_count must be greater than zero"
            )
        state = _DirectorySubscriptionStateMachine.apply_drop_report(self, cursor)
        next_metadata = dict(self.metadata)
        next_metadata["drop_reported"] = True
        next_metadata["snapshot_complete"] = True
        next_metadata.update(dict(metadata or {}))
        self._replace_projection(
            state=state,
            cursor=cursor,
            resume_token=cursor.resume_token(),
            drop_count=self.drop_count + dropped_count,
            metadata=next_metadata,
        )

    def mark_transport_lost(self) -> None:
        self._replace_projection(
            state=_DirectorySubscriptionStateMachine.transport_lost(self)
        )

    def mark_resume_ok(
        self, cursor: Optional[DirectorySubscriptionCursor] = None
    ) -> None:
        next_cursor = cursor or self.cursor
        _validate_subscription_cursor(next_cursor)
        self._replace_projection(
            state=_DirectorySubscriptionStateMachine.resume_ok(self, next_cursor),
            cursor=next_cursor,
            resume_token=next_cursor.resume_token(),
        )

    def mark_resume_failed(self) -> None:
        self._replace_projection(
            state=_DirectorySubscriptionStateMachine.resume_failed(self)
        )

    def next_event(
        self, timeout: float | None = None
    ) -> Optional[DirectorySubscriptionEvent]:
        if self._runtime_stream is None:
            raise _invalid_directory("directory subscription has no runtime stream")
        runtime_event = self._runtime_stream.next(timeout)
        if runtime_event.payload_json is None and runtime_event.terminal:
            self._replace_projection(state="Closed")
            return None
        payload = _mapping_object(runtime_event.payload_json, "directory stream payload")
        if payload.get("profile") == "events" and payload.get("kind") == "directory.drop_report":
            cursor = _subscription_cursor_from_json(payload.get("cursor"))
            dropped_count = _required_positive_int(payload, "dropped_count")
            self.apply_drop_report(
                cursor,
                dropped_count,
                metadata=_optional_mapping(payload.get("metadata"), "metadata") or {},
            )
            return None
        if payload.get("profile") == "events" and payload.get("kind") == "directory.terminal":
            cursor = _subscription_cursor_from_json(payload.get("cursor"))
            self._replace_projection(
                state="Closed",
                cursor=cursor,
                resume_token=cursor.resume_token(),
            )
            return None
        event = DirectorySubscriptionEvent.from_mapping(payload)
        return self.apply_event(event)

    def close(self) -> None:
        if self._runtime_stream is None:
            return
        self._runtime_stream.close()
        object.__setattr__(self, "state", "Closed")

    def _replace_projection(self, **changes: object) -> None:
        projection = replace(self, **changes)
        _validate_subscription(projection)
        for key, value in changes.items():
            object.__setattr__(self, key, value)


class _DirectorySubscriptionStateMachine:
    @staticmethod
    def apply_event(
        subscription: DirectorySubscription, event: DirectorySubscriptionEvent
    ) -> str:
        _require_non_terminal_subscription(subscription)
        if event.cursor.sequence <= subscription.cursor.sequence:
            raise _invalid_directory("directory subscription event sequence must advance cursor")
        if event.event_id in {buffered.event_id for buffered in subscription.events}:
            raise _invalid_directory("duplicate directory subscription event id")
        if event.terminal:
            return "Closed"
        if event.phase == "snapshot_start":
            if subscription.state not in {"Opening", "CatchingUp"}:
                raise _invalid_directory(
                    "snapshot_start requires Opening or CatchingUp subscription"
                )
            return "CatchingUp"
        if event.phase == "snapshot_complete":
            if subscription.state != "CatchingUp":
                raise _invalid_directory("snapshot_complete requires CatchingUp subscription")
            return "Live"
        if event.phase == "live":
            if subscription.state != "Live":
                raise _invalid_directory("live directory event requires Live subscription")
            return "Live"
        raise _invalid_directory("unknown directory subscription event phase")

    @staticmethod
    def apply_drop_report(
        subscription: DirectorySubscription, cursor: DirectorySubscriptionCursor
    ) -> str:
        _require_non_terminal_subscription(subscription)
        if subscription.state != "Live":
            raise _invalid_directory("directory drop report requires Live subscription")
        if cursor.sequence <= subscription.cursor.sequence:
            raise _invalid_directory("directory drop report cursor must advance")
        return "Resuming"

    @staticmethod
    def transport_lost(subscription: DirectorySubscription) -> str:
        _require_non_terminal_subscription(subscription)
        if subscription.state != "Live":
            raise _invalid_directory("transport_lost requires Live subscription")
        return "Resuming"

    @staticmethod
    def resume_ok(
        subscription: DirectorySubscription, cursor: DirectorySubscriptionCursor
    ) -> str:
        _require_non_terminal_subscription(subscription)
        if subscription.state != "Resuming":
            raise _invalid_directory("resume_ok requires Resuming subscription")
        if cursor.sequence < subscription.cursor.sequence:
            raise _invalid_directory("resume cursor must not move backwards")
        return "Live"

    @staticmethod
    def resume_failed(subscription: DirectorySubscription) -> str:
        _require_non_terminal_subscription(subscription)
        if subscription.state != "Resuming":
            raise _invalid_directory("resume_failed requires Resuming subscription")
        return "Failed"


@runtime_checkable
class DirectoryTransport(Protocol):
    """Concrete directory operations supplied by the integration layer."""

    def build_directory_subscription_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_list_devices_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_list_agents_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_list_abilities_invocation(self, request_json: bytes) -> bytes:
        ...

    def build_resolve_invocation(self, request_json: bytes) -> bytes:
        ...

    def project_device_page(self, page_json: bytes) -> bytes:
        ...

    def project_agent_page(self, page_json: bytes) -> bytes:
        ...

    def project_ability_page(self, page_json: bytes) -> bytes:
        ...

    def project_resolved_ref(self, answer_json: bytes) -> bytes:
        ...

    def project_subscription(self, subscription_json: bytes) -> bytes:
        ...

    def resolve(self, request_json: bytes) -> bytes:
        ...

    def list_devices(self, request_json: bytes) -> bytes:
        ...

    def list_agents(self, request_json: bytes) -> bytes:
        ...

    def list_abilities(self, request_json: bytes) -> bytes:
        ...

    def subscribe_directory(self, request_json: bytes) -> bytes | DirectorySubscription:
        ...


@dataclass(frozen=True)
class ResolvedRef:
    profile: str
    kind: str
    answer_kind: str
    query_name: Optional[str]
    canonical_name: Optional[str]
    owner_ura: Optional[str]
    ability_ura: Optional[str]
    route_ura: Optional[str]
    next_hop: Optional[Mapping[str, object]]
    selected_route: Optional[Mapping[str, object]]
    route_candidates: tuple[Mapping[str, object], ...]
    records: tuple[Mapping[str, object], ...]
    negative: Optional[Mapping[str, object]]
    release_profile: Optional[str]
    authority: Optional[Mapping[str, object]]
    cache_policy: Optional[Mapping[str, object]]
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str) -> "ResolvedRef":
        decoded = _json_object(raw, "resolved ref")
        if (
            decoded.get("profile") != _PROFILE
            or decoded.get("kind") != "resolved_ref"
            or not isinstance(decoded.get("answer_kind"), str)
        ):
            raise _invalid_directory("invalid directory resolved_ref projection")
        return cls(
            profile=decoded["profile"],
            kind=decoded["kind"],
            answer_kind=decoded["answer_kind"],
            query_name=_optional_string(decoded.get("query_name"), "query_name"),
            canonical_name=_optional_string(
                decoded.get("canonical_name"), "canonical_name"
            ),
            owner_ura=_optional_string(decoded.get("owner_ura"), "owner_ura"),
            ability_ura=_optional_string(decoded.get("ability_ura"), "ability_ura"),
            route_ura=_optional_string(decoded.get("route_ura"), "route_ura"),
            next_hop=_optional_mapping(decoded.get("next_hop"), "next_hop"),
            selected_route=_optional_mapping(
                decoded.get("selected_route"), "selected_route"
            ),
            route_candidates=_mapping_tuple(
                decoded.get("route_candidates", []), "route_candidates"
            ),
            records=_mapping_tuple(decoded.get("records", []), "records"),
            negative=_optional_mapping(decoded.get("negative"), "negative"),
            release_profile=_optional_string(
                decoded.get("release_profile"), "release_profile"
            ),
            authority=_optional_mapping(decoded.get("authority"), "authority"),
            cache_policy=_optional_mapping(decoded.get("cache_policy"), "cache_policy"),
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class DirectoryPage:
    """Typed page over a daemon directory read model."""

    profile: str
    kind: str
    item_kind: str
    items: tuple[Mapping[str, object], ...]
    next_cursor: Optional[str]
    limit: int
    source: str
    metadata: Mapping[str, object]

    @classmethod
    def from_json(cls, raw: bytes | str, *, kind: str, item_kind: str) -> "DirectoryPage":
        decoded = _json_object(raw, "directory page")
        if (
            decoded.get("profile") != _PROFILE
            or decoded.get("kind") != kind
            or decoded.get("item_kind") != item_kind
            or decoded.get("source") != _READ_MODEL
        ):
            raise _invalid_directory("invalid directory page projection")
        limit = _required_positive_int(decoded, "limit")
        if limit > MAX_DIRECTORY_PAGE_SIZE:
            raise _invalid_directory("directory page limit exceeds bounds")
        return cls(
            profile=decoded["profile"],
            kind=decoded["kind"],
            item_kind=decoded["item_kind"],
            items=_mapping_tuple(decoded.get("items", []), "items"),
            next_cursor=_optional_string(decoded.get("next_cursor"), "next_cursor"),
            limit=limit,
            source=decoded["source"],
            metadata=_required_mapping(decoded, "metadata"),
        )


@dataclass(frozen=True)
class DirectoryClient:
    """Directory + Identity read-model facade."""

    transport: DirectoryTransport
    _lifecycle: ClientLifecycle = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        if self.transport is None:
            raise _invalid_directory("directory transport is required")
        object.__setattr__(self, "_lifecycle", ClientLifecycle("directory"))

    def build_directory_subscription_invocation(
        self, request: DirectorySubscriptionRequest
    ) -> InvocationDraft:
        return self._build_invocation(
            request.to_json_bytes(),
            self.transport.build_directory_subscription_invocation,
            "directory subscription invocation failed",
        )

    def build_list_devices_invocation(self, query: DeviceQuery) -> InvocationDraft:
        return self._build_base_invocation(
            query.base,
            self.transport.build_list_devices_invocation,
            "directory list devices invocation failed",
        )

    def build_list_agents_invocation(self, query: AgentQuery) -> InvocationDraft:
        return self._build_base_invocation(
            query.base,
            self.transport.build_list_agents_invocation,
            "directory list agents invocation failed",
        )

    def build_list_abilities_invocation(self, query: AbilityQuery) -> InvocationDraft:
        return self._build_invocation(
            query.to_json_bytes(),
            self.transport.build_list_abilities_invocation,
            "directory list abilities invocation failed",
        )

    def build_resolve_invocation(self, query: ResolveQuery) -> InvocationDraft:
        return self._build_invocation(
            query.to_json_bytes(),
            self.transport.build_resolve_invocation,
            "directory resolve invocation failed",
        )

    def project_device_page(self, page_json: bytes) -> DirectoryPage:
        return self._project_page(
            page_json,
            self.transport.project_device_page,
            kind="device_page",
            item_kind="device",
            label="directory project device page failed",
        )

    def project_agent_page(self, page_json: bytes) -> DirectoryPage:
        return self._project_page(
            page_json,
            self.transport.project_agent_page,
            kind="agent_page",
            item_kind="agent",
            label="directory project agent page failed",
        )

    def project_ability_page(self, page_json: bytes) -> DirectoryPage:
        return self._project_page(
            page_json,
            self.transport.project_ability_page,
            kind="ability_page",
            item_kind="ability",
            label="directory project ability page failed",
        )

    def project_resolved_ref(self, answer_json: bytes) -> ResolvedRef:
        self._require_open()
        try:
            raw = self.transport.project_resolved_ref(answer_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("directory project resolved ref failed", exc) from exc
        return ResolvedRef.from_json(raw)

    def project_subscription(self, subscription_json: bytes) -> DirectorySubscription:
        self._require_open()
        try:
            raw = self.transport.project_subscription(subscription_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("directory project subscription failed", exc) from exc
        return DirectorySubscription.from_json(raw)

    def subscribe_directory(self, request: DirectorySubscriptionRequest) -> DirectorySubscription:
        self._require_open()
        try:
            raw = self.transport.subscribe_directory(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("directory subscribe failed", exc) from exc
        if isinstance(raw, DirectorySubscription):
            return raw
        return DirectorySubscription.from_json(raw)

    def resolve(self, query: ResolveQuery) -> ResolvedRef:
        self._require_open()
        try:
            raw = self.transport.resolve(query.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("directory resolve failed", exc) from exc
        return ResolvedRef.from_json(raw)

    def list_devices(self, query: DeviceQuery) -> DirectoryPage:
        return self._list_page(
            query.base,
            self.transport.list_devices,
            kind="device_page",
            item_kind="device",
            label="directory list devices failed",
        )

    def list_agents(self, query: AgentQuery) -> DirectoryPage:
        return self._list_page(
            query.base,
            self.transport.list_agents,
            kind="agent_page",
            item_kind="agent",
            label="directory list agents failed",
        )

    def list_abilities(self, query: AbilityQuery) -> DirectoryPage:
        self._require_open()
        try:
            raw = self.transport.list_abilities(query.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("directory list abilities failed", exc) from exc
        return DirectoryPage.from_json(raw, kind="ability_page", item_kind="ability")

    def _list_page(
        self,
        base: DirectoryQueryBase,
        fn: Callable[[bytes], bytes],
        *,
        kind: str,
        item_kind: str,
        label: str,
    ) -> DirectoryPage:
        self._require_open()
        try:
            raw = fn(_base_page_json(base))
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        return DirectoryPage.from_json(raw, kind=kind, item_kind=item_kind)

    def _build_base_invocation(
        self,
        base: DirectoryQueryBase,
        fn: Callable[[bytes], bytes],
        label: str,
    ) -> InvocationDraft:
        return self._build_invocation(_base_page_json(base), fn, label)

    def _build_invocation(
        self,
        request_json: bytes,
        fn: Callable[[bytes], bytes],
        label: str,
    ) -> InvocationDraft:
        self._require_open()
        try:
            raw = fn(request_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        return InvocationDraft.from_json(raw)

    def _project_page(
        self,
        page_json: bytes,
        fn: Callable[[bytes], bytes],
        *,
        kind: str,
        item_kind: str,
        label: str,
    ) -> DirectoryPage:
        self._require_open()
        try:
            raw = fn(page_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(label, exc) from exc
        return DirectoryPage.from_json(raw, kind=kind, item_kind=item_kind)

    def close(self) -> None:
        self._lifecycle.close(self.transport)

    def _require_open(self) -> None:
        self._lifecycle.require_open()


def _base_page_json(base: DirectoryQueryBase) -> bytes:
    bounded = base.with_default_limit()
    _validate_base(bounded, require_limit=True)
    return _json_bytes(bounded.to_json_dict())


def _validate_base(base: DirectoryQueryBase, *, require_limit: bool) -> None:
    if (
        not base.caller_ura
        or not base.callee_ura
        or not base.subject_ura
        or not base.descriptor_version
        or not base.nonce_base64
    ):
        raise _invalid_directory(
            "caller_ura, callee_ura, subject_ura, descriptor_version, and nonce_base64 are required"
        )
    if base.causal_context is None:
        raise _invalid_directory("causal_context is required")
    if require_limit and (base.limit < 1 or base.limit > MAX_DIRECTORY_PAGE_SIZE):
        raise _invalid_directory("directory page limit exceeds bounds")


def _validate_subscription(subscription: DirectorySubscription) -> None:
    if (
        subscription.profile != _PROFILE
        or subscription.kind != "directory_subscription"
        or subscription.stream != _DIRECTORY_STREAM
        or subscription.state not in _DIRECTORY_SUBSCRIPTION_STATES
    ):
        raise _invalid_directory("invalid directory subscription projection")
    _validate_subscription_cursor(subscription.cursor)
    if subscription.resume_token != subscription.cursor.resume_token():
        raise _invalid_directory("directory subscription resume token mismatch")
    if subscription.drop_count < 0:
        raise _invalid_directory("directory subscription drop_count must be non-negative")
    seen: set[str] = set()
    snapshot_complete = _allows_truncated_subscription_buffer(subscription)
    last_sequence = -1
    for event in subscription.events:
        if event.event_id in seen:
            raise _invalid_directory("duplicate directory subscription event id")
        seen.add(event.event_id)
        if event.cursor.sequence <= last_sequence:
            raise _invalid_directory("directory subscription event sequence must increase")
        last_sequence = event.cursor.sequence
        if event.phase == "live" and not snapshot_complete:
            raise _invalid_directory("directory live event before snapshot_complete")
        if event.phase == "snapshot_complete":
            snapshot_complete = True
        if event.phase not in _DIRECTORY_EVENT_PHASES:
            raise _invalid_directory("unknown directory subscription event phase")
    if last_sequence >= 0 and subscription.cursor.sequence < last_sequence:
        raise _invalid_directory("directory subscription cursor must cover buffered events")


def _validate_subscription_event(event: DirectorySubscriptionEvent) -> None:
    if event.profile != _PROFILE or event.stream != _DIRECTORY_STREAM:
        raise _invalid_directory("invalid directory subscription event projection")
    _validate_subscription_cursor(event.cursor)
    if event.resume_token != event.cursor.resume_token():
        raise _invalid_directory("directory subscription event resume token mismatch")
    if event.phase not in _DIRECTORY_EVENT_PHASES:
        raise _invalid_directory("unknown directory subscription event phase")
    if event.terminal and event.phase != "terminal":
        raise _invalid_directory("terminal directory event must use terminal phase")


def _validate_subscription_cursor(cursor: DirectorySubscriptionCursor) -> None:
    if cursor.stream != _DIRECTORY_STREAM:
        raise _invalid_directory("directory subscription cursor stream mismatch")
    if cursor.sequence < 0:
        raise _invalid_directory("directory subscription cursor sequence must be non-negative")
    token = cursor.resume_token()
    if not token or any(ch.isspace() for ch in token):
        raise _invalid_directory("directory subscription cursor token is invalid")
    if token != f"{cursor.stream}:{cursor.sequence}":
        raise _invalid_directory("directory subscription cursor token mismatch")


def _subscription_cursor_from_json(value: object) -> DirectorySubscriptionCursor:
    if not isinstance(value, dict):
        raise _invalid_directory("directory subscription cursor must be an object")
    cursor = DirectorySubscriptionCursor(
        stream=_required_string(value, "stream"),
        sequence=_required_non_negative_int(value, "sequence"),
        token=_optional_string(value.get("token"), "token") or "",
    )
    _validate_subscription_cursor(cursor)
    return cursor


def _require_non_terminal_subscription(subscription: DirectorySubscription) -> None:
    if subscription.state in _DIRECTORY_SUBSCRIPTION_TERMINAL_STATES:
        raise _invalid_directory("directory subscription is terminal")


def _allows_truncated_subscription_buffer(subscription: DirectorySubscription) -> bool:
    return (
        subscription.drop_count > 0
        or subscription.metadata.get("snapshot_complete") is True
        or subscription.metadata.get("source") == "runtime_stream"
    )


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _json_object(raw: bytes | str, label: str) -> dict[str, object]:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_directory(f"decode {label} JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_directory(f"{label} JSON must be an object")
    return decoded


def _required_mapping(decoded: Mapping[str, object], field_name: str) -> Mapping[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise _invalid_directory(f"{field_name} must be an object")
    return dict(value)


def _mapping_object(value: object, field_name: str) -> Mapping[str, object]:
    if not isinstance(value, dict):
        raise _invalid_directory(f"{field_name} must be an object")
    return dict(value)


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_directory(f"{field_name} is required")
    return value


def _optional_mapping(value: object, field_name: str) -> Optional[Mapping[str, object]]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_directory(f"{field_name} must be an object or null")
    return dict(value)


def _mapping_tuple(value: object, field_name: str) -> tuple[Mapping[str, object], ...]:
    if not isinstance(value, list) or not all(isinstance(item, dict) for item in value):
        raise _invalid_directory(f"{field_name} must be an array of objects")
    return tuple(dict(item) for item in value)


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_directory(f"{field_name} must be a string or null")
    return value


def _required_non_negative_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_directory(f"{field_name} must be a non-negative integer")
    return value


def _required_positive_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise _invalid_directory(f"{field_name} must be a positive integer")
    return value


def _required_bool(decoded: Mapping[str, object], field_name: str) -> bool:
    value = decoded.get(field_name)
    if not isinstance(value, bool):
        raise _invalid_directory(f"{field_name} must be a boolean")
    return value


def _invalid_directory(
    message: str, cause: BaseException | None = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="directory_identity",
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
