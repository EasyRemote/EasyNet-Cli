from __future__ import annotations

import json

import pytest

from easynet_sdk.axon_addressing import AddressingClient, AxonAddressingTransport
from easynet_sdk.directory import (
    DirectoryCursor,
    DirectoryListRequest,
    DirectoryResolveKind,
    DirectoryResolveRequest,
    DirectorySubscribeRequest,
    DirectorySubscription,
    DirectorySubscriptionState,
    RuntimeDirectoryProvider,
)
from easynet_sdk.errors import SDKError
from easynet_sdk.runtime import RuntimeClient
from easynet_sdk.runtime_ability import RuntimeAbilityClient
from easynet_sdk.stream import StreamHandle

from test_runtime_ability import RuntimeTransportFake, _call


def _provider() -> tuple[RuntimeDirectoryProvider, RuntimeTransportFake]:
    transport = RuntimeTransportFake()
    ability = RuntimeAbilityClient(
        RuntimeClient(transport),  # type: ignore[arg-type]
        AddressingClient(AxonAddressingTransport()),
    )
    return RuntimeDirectoryProvider(ability), transport


def test_runtime_directory_resolves_through_canonical_ability() -> None:
    provider, transport = _provider()
    transport.output_json = {
        "answer": {
            "answer_kind": "positive",
            "next_hop": {"node_id": "node-1"},
            "selected_route": {"route_ura": "easynet:///r/example/device/node-1"},
            "route_candidates": [{"node_id": "node-1"}],
            "records": [],
        }
    }
    resolution = provider.resolve(
        DirectoryResolveRequest(
            call=_call(),
            query_ura="easynet:///r/example/user/alice",
            kind=DirectoryResolveKind.CANONICAL_IDENTITY,
            include_abilities=False,
        )
    )
    assert resolution.answer_kind == "positive"
    assert resolution.next_hop == {"node_id": "node-1"}
    assert resolution.route_candidates == ({"node_id": "node-1"},)
    assert transport.seen["args"] == {
        "query_name": "easynet:///r/example/user/alice",
        "qtype": "RESOLVE_TYPE_CANONICAL_IDENTITY",
        "include_abilities": False,
    }


def test_runtime_directory_list_forwards_and_validates_cursor() -> None:
    provider, transport = _provider()
    transport.output_json = {
        "answer_kind": "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
        "canonical_name": "easynet:///r/example/user/alice",
        "records": [
            {
                "name": "easynet:///r/example/user/alice",
                "record_type": "RECORD_TYPE_ID",
                "value": {"id": {"ura": "easynet:///r/example/user/alice"}},
            }
        ],
        "next_cursor": "directory:v1:cursor-2",
    }
    page = provider.list(
        DirectoryListRequest(
            call=_call(),
            ura_prefix="easynet:///r/example/user/alice",
            limit=1,
            cursor=" directory:v1:cursor-1 ",
            include_abilities=False,
        )
    )
    assert len(page.records) == 1
    assert page.next_cursor == "directory:v1:cursor-2"
    assert transport.seen["args"] == {
        "qtype": "RESOLVE_TYPE_DIRECTORY_LISTING",
        "query_name": "easynet:///r/example/user/alice",
        "limit": 1,
        "cursor": "directory:v1:cursor-1",
        "include_abilities": False,
    }

    transport.output_json = {
        "answer_kind": "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
        "records": [],
        "next_cursor": "directory:v1:cursor-1",
    }
    with pytest.raises(SDKError, match="repeated cursor"):
        provider.list(DirectoryListRequest(call=_call(), ura_prefix="", cursor="directory:v1:cursor-1"))

    with pytest.raises(SDKError, match="cursor exceeds"):
        provider.list(DirectoryListRequest(call=_call(), ura_prefix="", cursor="x" * 4097))


def test_runtime_directory_list_rejects_negative_listing_answer() -> None:
    provider, transport = _provider()
    transport.output_json = {
        "answer_kind": "RESOLVE_ANSWER_KIND_NEGATIVE",
        "records": [],
        "negative": {
            "reason": "NEGATIVE_REASON_REFUSED",
            "detail": "namespace.resolve Directory cursor does not match the current query",
        },
    }
    with pytest.raises(SDKError, match="cursor does not match"):
        provider.list(DirectoryListRequest(call=_call(), ura_prefix=""))


def test_runtime_directory_forwards_subscription_resume_cursor() -> None:
    transport = RuntimeDirectoryStreamTransportFake()
    ability = RuntimeAbilityClient(
        RuntimeClient(transport),  # type: ignore[arg-type]
        AddressingClient(AxonAddressingTransport()),
    )
    provider = RuntimeDirectoryProvider(ability)
    provider.subscribe(
        DirectorySubscribeRequest(call=_call(), resume_cursor=DirectoryCursor.at(4))
    )
    assert transport.seen["args"] == {
        "resume_sequence": 4,
        "resume_token": "directory:4",
    }


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


class RuntimeDirectoryStreamTransportFake(RuntimeTransportFake):
    def open_stream(self, draft_json: bytes):
        self.seen = json.loads(draft_json)
        return (
            DirectoryStreamTransport([]),
            b'{"stream_id":"directory-1","state":"Opening","max_buffered_events":8}',
        )


def _stream(events: list[dict[str, object]]) -> StreamHandle:
    return StreamHandle.from_json(
        DirectoryStreamTransport(events),
        b'{"stream_id":"directory-1","state":"Opening","max_buffered_events":8}',
    )


def _stream_event(sequence: int, payload: dict[str, object]) -> dict[str, object]:
    return {
        "sequence": sequence,
        "event": "chunk",
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
    assert first.cursor == DirectoryCursor.at(1)
    assert subscription.state == DirectorySubscriptionState.LIVE
    second = subscription.next()
    assert second.event is not None and second.event.type == "heartbeat"
    assert second.cursor.sequence == 2


def test_directory_subscription_fails_on_delta_before_snapshot() -> None:
    subscription = DirectorySubscription(
        _stream([_stream_event(1, {"type": "heartbeat", "unix_ms": 1})])
    )
    with pytest.raises(SDKError, match="requires snapshot as frame zero"):
        subscription.next()
    assert subscription.state == DirectorySubscriptionState.FAILED
