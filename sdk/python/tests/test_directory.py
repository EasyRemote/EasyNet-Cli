from __future__ import annotations

import json

import pytest

from easynet_sdk.axon_addressing import AddressingClient, AxonAddressingTransport
from easynet_sdk.directory import (
    DirectoryCursor,
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
    resolution = provider.resolve(
        DirectoryResolveRequest(
            call=_call(),
            query_ura="easynet:///r/example/user/alice",
            kind=DirectoryResolveKind.CANONICAL_IDENTITY,
        )
    )
    assert resolution.answer_kind == "positive"
    assert transport.seen["args"] == {
        "query_name": "easynet:///r/example/user/alice",
        "qtype": "RESOLVE_TYPE_CANONICAL_IDENTITY",
    }


def test_runtime_directory_keeps_resume_seam_explicit() -> None:
    provider, _ = _provider()
    with pytest.raises(SDKError, match="resume provider is not available"):
        provider.subscribe(
            DirectorySubscribeRequest(call=_call(), resume_cursor=DirectoryCursor.at(4))
        )


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
