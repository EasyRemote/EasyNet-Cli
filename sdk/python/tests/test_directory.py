from __future__ import annotations

import json

import pytest

import easynet_sdk.directory as directory_module
from easynet_sdk.directory import (
    DirectoryClient,
    DirectoryCursor,
    DirectoryListRequest,
    DirectoryPage,
    DirectoryResolveKind,
    DirectoryResolveRequest,
    DirectoryResolution,
    DirectorySubscribeRequest,
    DirectorySubscription,
    DirectorySubscriptionState,
    parse_directory_event,
)
from easynet_sdk.errors import SDKError
from easynet_sdk.stream import StreamHandle

from test_runtime_ability import _call


class FakeDirectoryProvider:
    def __init__(self) -> None:
        self.resolve_request: DirectoryResolveRequest | None = None
        self.list_request: DirectoryListRequest | None = None
        self.subscribe_request: DirectorySubscribeRequest | None = None
        self.resolution = DirectoryResolution(
            answer_kind="RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
            canonical_ura="easynet:///r/example/user/alice",
            records=(),
        )

    def resolve(self, request: DirectoryResolveRequest) -> DirectoryResolution:
        self.resolve_request = request
        return self.resolution

    def list(self, request: DirectoryListRequest) -> DirectoryPage:
        self.list_request = request
        return DirectoryPage(records=(), limit=request.limit)

    def subscribe(self, request: DirectorySubscribeRequest) -> DirectorySubscription:
        self.subscribe_request = request
        return DirectorySubscription(_stream([]))


def test_directory_client_delegates_resolve_to_injected_provider() -> None:
    provider = FakeDirectoryProvider()
    client = DirectoryClient(provider)

    resolution = client.resolve(
        DirectoryResolveRequest(
            call=_call(),
            query_ura="easynet:///r/example/user/alice",
            kind=DirectoryResolveKind.CANONICAL_IDENTITY,
            include_abilities=False,
        )
    )

    assert resolution.canonical_ura == "easynet:///r/example/user/alice"
    assert provider.resolve_request is not None
    assert provider.resolve_request.query_ura == "easynet:///r/example/user/alice"
    assert provider.resolve_request.kind == DirectoryResolveKind.CANONICAL_IDENTITY
    assert provider.resolve_request.include_abilities is False


def test_project_directory_resolution_preserves_resolver_facts() -> None:
    resolution = directory_module._project_resolution(
        {
            "answer": {
                "answer_kind": "positive",
                "next_hop": {"node_id": "node-1"},
                "selected_route": {"route_ura": "easynet:///r/example/device/node-1"},
                "route_candidates": [{"node_id": "node-1"}],
                "records": [
                    {
                        "kind": "ID",
                        "ura": "easynet:///r/example/user/alice",
                    }
                ],
            }
        }
    )

    assert resolution.answer_kind == "positive"
    assert resolution.next_hop == {"node_id": "node-1"}
    assert resolution.route_candidates == ({"node_id": "node-1"},)
    assert len(resolution.records) == 1


def test_project_directory_resolution_rejects_malformed_present_facts() -> None:
    base = {
        "answer_kind": "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
        "records": [],
    }
    cases = [
        ("records", "not-a-list", "records must be a list"),
        ("next_hop", "not-an-object", "next_hop must be an object"),
        ("selected_route", "not-an-object", "selected_route must be an object"),
        ("route_candidates", {"node_id": "node-1"}, "route_candidates must be a list"),
        (
            "route_candidates",
            ["not-an-object"],
            "route_candidates item must be an object",
        ),
        ("negative", "not-an-object", "negative must be an object"),
        ("authority", "not-an-object", "authority must be an object"),
        ("cache_policy", "not-an-object", "cache_policy must be an object"),
    ]
    for key, value, message in cases:
        payload = dict(base)
        payload[key] = value
        with pytest.raises(SDKError, match=message):
            directory_module._project_resolution(payload)


def test_directory_helpers_reject_unbounded_cursor_and_surface_negative_detail() -> None:
    with pytest.raises(SDKError, match="cursor exceeds"):
        directory_module._directory_cursor("x" * 4097)
    with pytest.raises(SDKError, match="limit exceeds"):
        directory_module._directory_limit(501)
    detail = directory_module._negative_detail(
        DirectoryResolution(
            answer_kind="RESOLVE_ANSWER_KIND_NEGATIVE",
            records=(),
            negative={
                "reason": "NEGATIVE_REASON_REFUSED",
                "detail": "Directory cursor does not match the current query",
            },
        )
    )
    assert "cursor does not match" in detail


class DirectoryStreamTransport:
    def __init__(self, events: list[dict[str, object]]) -> None:
        self._events = events
        self.closed = False

    def recv(self, timeout: float | None = None) -> bytes:
        del timeout
        return json.dumps(self._events.pop(0)).encode()

    def cancel(self, reason: str) -> bytes:
        del reason
        return b'{"stream_id":"directory-1","cancelled":true,"state":"Cancelled","terminal":true}'

    def close(self) -> None:
        self.closed = True


def _stream(events: list[dict[str, object]]) -> StreamHandle:
    return StreamHandle.from_json(
        DirectoryStreamTransport(events),
        b'{"stream_id":"directory-1","state":"Opening","max_buffered_events":8}',
    )


def _stream_event(sequence: int, payload: dict[str, object]) -> dict[str, object]:
    return {
        "sequence": sequence,
        "kind": "data",
        "state": "Open",
        "terminal": False,
        "payload_content_type": "application/json",
        "payload_json": payload,
    }


def test_directory_subscription_requires_snapshot_then_live_deltas() -> None:
    subscription = DirectorySubscription(
        _stream(
            [
                _stream_event(1, {"type": "snapshot", "agents": [], "snapshot_unix_ms": 1}),
                _stream_event(2, {"type": "heartbeat", "unix_ms": 2}),
            ]
        )
    )
    first = subscription.next()
    assert first.event is not None and first.event.type == "snapshot"
    assert first.event.raw["snapshot_unix_ms"] == 1
    assert first.cursor == DirectoryCursor.at(1)
    assert subscription.state == DirectorySubscriptionState.LIVE
    second = subscription.next()
    assert second.event is not None and second.event.type == "heartbeat"
    assert second.event.raw["unix_ms"] == 2
    assert second.cursor.sequence == 2


def test_directory_subscription_fails_on_delta_before_snapshot() -> None:
    subscription = DirectorySubscription(
        _stream([_stream_event(1, {"type": "heartbeat", "unix_ms": 1})])
    )
    with pytest.raises(SDKError, match="requires snapshot as frame zero"):
        subscription.next()
    assert subscription.state == DirectorySubscriptionState.FAILED


def test_directory_event_projection_is_runtime_generic() -> None:
    event = parse_directory_event(
        {
            "type": "snapshot",
            "agents": [
                {
                    "agent_ura": "easynet:///r/example/agent/alpha",
                    "signing_authority": {
                        "kind": "hosted_by",
                        "host_ura": "easynet:///r/example/device/node-1",
                    },
                    "status": "online",
                    "ability_count": 3,
                }
            ],
            "snapshot_unix_ms": 42,
        }
    )

    assert event.type == "snapshot"
    assert event.raw["agents"] == [
        {
            "agent_ura": "easynet:///r/example/agent/alpha",
            "signing_authority": {
                "kind": "hosted_by",
                "host_ura": "easynet:///r/example/device/node-1",
            },
            "status": "online",
            "ability_count": 3,
        }
    ]

    with pytest.raises(SDKError, match="type is required"):
        parse_directory_event(
            {
                "agent_ura": "easynet:///r/example/agent/alpha",
            }
        )

    with pytest.raises(SDKError, match="event must be an object"):
        parse_directory_event(["not", "an", "object"])
