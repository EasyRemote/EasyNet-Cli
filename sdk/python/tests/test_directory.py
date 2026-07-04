import json
import unittest

from easynet_sdk import (
    DEFAULT_DIRECTORY_PAGE_SIZE,
    MAX_DIRECTORY_PAGE_SIZE,
    AbilityQuery,
    AgentQuery,
    DeviceQuery,
    DirectoryClient,
    DirectoryQueryBase,
    DirectorySubscription,
    DirectorySubscriptionCursor,
    DirectorySubscriptionEvent,
    DirectorySubscriptionRequest,
    ErrorCode,
    ResolveQuery,
    SDKError,
    StreamHandle,
    is_code,
)


class MemoryDirectoryTransport:
    def __init__(self) -> None:
        self.resolve_json = b"{}"
        self.devices_json = b"{}"
        self.agents_json = b"{}"
        self.abilities_json = b"{}"
        self.subscription_invocation_json = b"{}"
        self.list_devices_invocation_json = b"{}"
        self.list_agents_invocation_json = b"{}"
        self.list_abilities_invocation_json = b"{}"
        self.resolve_invocation_json = b"{}"
        self.subscription_json = b"{}"
        self.seen_request: dict[str, object] | None = None
        self.close_calls = 0

    def build_directory_subscription_invocation(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.subscription_invocation_json

    def build_list_devices_invocation(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.list_devices_invocation_json

    def build_list_agents_invocation(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.list_agents_invocation_json

    def build_list_abilities_invocation(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.list_abilities_invocation_json

    def build_resolve_invocation(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.resolve_invocation_json

    def project_device_page(self, page_json: bytes) -> bytes:
        self.seen_request = json.loads(page_json.decode("utf-8"))
        return self.devices_json

    def project_agent_page(self, page_json: bytes) -> bytes:
        self.seen_request = json.loads(page_json.decode("utf-8"))
        return self.agents_json

    def project_ability_page(self, page_json: bytes) -> bytes:
        self.seen_request = json.loads(page_json.decode("utf-8"))
        return self.abilities_json

    def project_resolved_ref(self, answer_json: bytes) -> bytes:
        self.seen_request = json.loads(answer_json.decode("utf-8"))
        return self.resolve_json

    def resolve(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.resolve_json

    def list_devices(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.devices_json

    def list_agents(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.agents_json

    def list_abilities(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.abilities_json

    def subscribe_directory(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.subscription_json

    def close(self) -> None:
        self.close_calls += 1


class MemoryStreamTransport:
    def __init__(self, events: list[bytes]) -> None:
        self.events = list(events)
        self.closed = False

    def recv(self, timeout: float | None = None) -> bytes:
        if not self.events:
            raise RuntimeError("no event")
        return self.events.pop(0)

    def cancel(self, reason: str) -> bytes:
        return b'{"stream_id":"stream-1","cancelled":true,"state":"Cancelled","terminal":true}'

    def close(self) -> None:
        self.closed = True


def base_query(limit: int = 0) -> DirectoryQueryBase:
    return DirectoryQueryBase(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/device/dev-a",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        cursor="0",
        limit=limit,
        metadata={"request_id": "directory-test"},
    )


def subscription_event(
    sequence: int,
    event_id: str,
    phase: str,
    *,
    terminal: bool = False,
) -> DirectorySubscriptionEvent:
    return DirectorySubscriptionEvent.from_mapping(
        {
            "profile": "directory_identity",
            "stream": "directory",
            "kind": phase,
            "event_id": event_id,
            "phase": phase,
            "cursor": {
                "stream": "directory",
                "sequence": sequence,
                "token": f"directory:{sequence}",
            },
            "resume_token": f"directory:{sequence}",
            "terminal": terminal,
            "metadata": {"source": "directory.subscribe"},
        }
    )


def subscription_event_payload(event: DirectorySubscriptionEvent) -> dict[str, object]:
    payload: dict[str, object] = {
        "profile": event.profile,
        "stream": event.stream,
        "kind": event.kind,
        "event_id": event.event_id,
        "phase": event.phase,
        "cursor": event.cursor.to_json_dict(),
        "resume_token": event.resume_token,
        "terminal": event.terminal,
        "metadata": dict(event.metadata),
    }
    if event.item_kind:
        payload["item_kind"] = event.item_kind
    if event.item is not None:
        payload["item"] = dict(event.item)
    return payload


def stream_frame(sequence: int, payload: dict[str, object], *, terminal: bool = False) -> bytes:
    return json.dumps(
        {
            "sequence": sequence,
            "event": payload.get("kind", "directory"),
            "state": "Open",
            "terminal": terminal,
            "payload_content_type": "application/json",
            "payload_json": payload,
        },
        separators=(",", ":"),
    ).encode("utf-8")


class DirectoryTests(unittest.TestCase):
    def test_subscription_builds_invocation_and_state_projection(self) -> None:
        transport = MemoryDirectoryTransport()
        transport.subscription_invocation_json = DIRECTORY_SUBSCRIPTION_INVOCATION_JSON
        transport.subscription_json = DIRECTORY_SUBSCRIPTION_JSON
        client = DirectoryClient(transport)

        draft = client.build_directory_subscription_invocation(
            DirectorySubscriptionRequest(
                base_query(),
                owner_ura="easynet:///r/example/device/dev-a",
                item_kind="ability",
                resume_cursor=DirectorySubscriptionCursor("directory", 1),
            )
        )

        self.assertEqual(
            draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.directory.subscribe@1.0.0",
        )
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["stream"], "directory")
        self.assertEqual(transport.seen_request["resume_cursor"]["token"], "directory:1")

        subscription = client.subscribe_directory(
            DirectorySubscriptionRequest(
                base_query(),
                device_ura="easynet:///r/example/device/dev-a",
                item_kind="ability",
            )
        )

        self.assertEqual(subscription.state, "Live")
        self.assertEqual(subscription.resume_token, "directory:3")
        self.assertEqual(len(subscription.events), 3)
        self.assertEqual(subscription.events[2].phase, "live")
        self.assertEqual(subscription.events[2].item_kind, "ability")

    def test_subscription_state_machine_applies_buffered_events_drop_and_resume(self) -> None:
        stream = StreamHandle.from_json(
            MemoryStreamTransport([]),
            b'{"stream_id":"directory-stream","state":"Opening","max_buffered_events":4}',
        )
        subscription = DirectorySubscription.from_runtime_stream(
            stream,
            cursor=DirectorySubscriptionCursor("directory", 0),
        )

        subscription.apply_event(subscription_event(1, "evt-1", "snapshot_start"))
        subscription.apply_event(subscription_event(2, "evt-2", "snapshot_complete"))
        subscription.apply_event(subscription_event(3, "evt-3", "live"))
        subscription.apply_drop_report(
            DirectorySubscriptionCursor("directory", 7),
            4,
            metadata={"reason": "consumer_lagged"},
        )
        subscription.mark_resume_ok(DirectorySubscriptionCursor("directory", 7))
        subscription.apply_event(subscription_event(8, "evt-8", "live"))

        self.assertEqual(subscription.state, "Live")
        self.assertEqual(subscription.cursor.resume_token(), "directory:8")
        self.assertEqual(subscription.drop_count, 4)
        self.assertEqual(
            [event.event_id for event in subscription.events],
            ["evt-1", "evt-2", "evt-3", "evt-8"],
        )
        self.assertTrue(subscription.metadata["drop_reported"])

    def test_subscription_next_event_projects_runtime_stream_payloads(self) -> None:
        drop_report = {
            "profile": "events",
            "stream": "directory",
            "kind": "directory.drop_report",
            "event_id": "evt-drop",
            "cursor": {"stream": "directory", "sequence": 4, "token": "directory:4"},
            "resume_token": "resnapshot",
            "dropped_count": 2,
            "terminal": False,
            "metadata": {"reason": "consumer_lagged"},
        }
        terminal = {
            "profile": "events",
            "stream": "directory",
            "kind": "directory.terminal",
            "event_id": "evt-terminal",
            "cursor": {"stream": "directory", "sequence": 5, "token": "directory:5"},
            "resume_token": "terminal",
            "dropped_count": 0,
            "terminal": True,
            "metadata": {"reason": "client_closed"},
        }
        stream = StreamHandle.from_json(
            MemoryStreamTransport(
                [
                    stream_frame(
                        1,
                        subscription_event_payload(
                            subscription_event(1, "evt-1", "snapshot_start")
                        ),
                    ),
                    stream_frame(
                        2,
                        subscription_event_payload(
                            subscription_event(2, "evt-2", "snapshot_complete")
                        ),
                    ),
                    stream_frame(
                        3,
                        subscription_event_payload(
                            subscription_event(3, "evt-3", "live")
                        ),
                    ),
                    stream_frame(4, drop_report),
                    stream_frame(5, terminal, terminal=True),
                ]
            ),
            b'{"stream_id":"directory-stream","state":"Opening","max_buffered_events":8}',
        )
        subscription = DirectorySubscription.from_runtime_stream(
            stream,
            cursor=DirectorySubscriptionCursor("directory", 0),
        )

        first = subscription.next_event()
        second = subscription.next_event()
        third = subscription.next_event()
        drop = subscription.next_event()
        closed = subscription.next_event()

        self.assertEqual(first.event_id, "evt-1")
        self.assertEqual(second.phase, "snapshot_complete")
        self.assertEqual(third.phase, "live")
        self.assertIsNone(drop)
        self.assertIsNone(closed)
        self.assertEqual(subscription.state, "Closed")
        self.assertEqual(subscription.cursor.resume_token(), "directory:5")
        self.assertEqual(subscription.drop_count, 2)

    def test_subscription_state_machine_rejects_duplicate_and_cursor_regression(self) -> None:
        stream = StreamHandle.from_json(
            MemoryStreamTransport([]),
            b'{"stream_id":"directory-stream","state":"Opening","max_buffered_events":4}',
        )
        subscription = DirectorySubscription.from_runtime_stream(
            stream,
            cursor=DirectorySubscriptionCursor("directory", 0),
        )
        subscription.apply_event(subscription_event(1, "evt-1", "snapshot_start"))

        with self.assertRaises(SDKError) as caught:
            subscription.apply_event(subscription_event(1, "evt-1", "snapshot_start"))

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(subscription.state, "Failed")

    def test_directory_carrier_builders_delegate_to_transport(self) -> None:
        transport = MemoryDirectoryTransport()
        transport.list_devices_invocation_json = invocation_json(
            "easynet:///r/example/ability/device.dev-a.directory.list-devices@1.0.0"
        )
        transport.list_agents_invocation_json = invocation_json(
            "easynet:///r/example/ability/device.dev-a.directory.list-agents@1.0.0"
        )
        transport.list_abilities_invocation_json = invocation_json(
            "easynet:///r/example/ability/device.dev-a.directory.list-abilities@1.0.0"
        )
        transport.resolve_invocation_json = invocation_json(
            "easynet:///r/example/ability/device.dev-a.directory.resolve@1.0.0"
        )
        client = DirectoryClient(transport)

        devices = client.build_list_devices_invocation(DeviceQuery(base_query()))

        self.assertEqual(
            devices.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.directory.list-devices@1.0.0",
        )
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["limit"], DEFAULT_DIRECTORY_PAGE_SIZE)

        agents = client.build_list_agents_invocation(AgentQuery(base_query(10)))

        self.assertEqual(
            agents.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.directory.list-agents@1.0.0",
        )
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["limit"], 10)

        abilities = client.build_list_abilities_invocation(
            AbilityQuery(base_query(5), scope="local", owner_ura="owner-1")
        )

        self.assertEqual(
            abilities.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.directory.list-abilities@1.0.0",
        )
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["scope"], "local")
        self.assertEqual(transport.seen_request["owner_ura"], "owner-1")

        resolved = client.build_resolve_invocation(
            ResolveQuery(base_query(), query_name="easynet:///r/example/device/dev-a")
        )

        self.assertEqual(
            resolved.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.directory.resolve@1.0.0",
        )
        assert transport.seen_request is not None
        self.assertEqual(
            transport.seen_request["query_name"],
            "easynet:///r/example/device/dev-a",
        )

    def test_subscription_rejects_invalid_state_transitions(self) -> None:
        from easynet_sdk import DirectorySubscription

        with self.assertRaises(SDKError):
            DirectorySubscription.from_json(DIRECTORY_SUBSCRIPTION_LIVE_BEFORE_SNAPSHOT_JSON)
        with self.assertRaises(SDKError):
            DirectorySubscription.from_json(DIRECTORY_SUBSCRIPTION_DUPLICATE_EVENT_JSON)

    def test_list_devices_defaults_bounded_page_size(self) -> None:
        transport = MemoryDirectoryTransport()
        transport.devices_json = json.dumps(
            {
                "profile": "directory_identity",
                "kind": "device_page",
                "item_kind": "device",
                "items": [
                    {
                        "profile": "directory_identity",
                        "kind": "device",
                        "node_id": "dev-a",
                        "device_ura": "easynet:///r/example/device/dev-a",
                        "state": "online",
                        "online": True,
                        "is_self": True,
                        "paired": True,
                        "tenant_id": "tenant-a",
                        "hub_endpoint": "https://hub.example",
                        "probe_status": "ok",
                        "probe_error": None,
                        "latency_ms": 12,
                        "abilities": [],
                        "metadata": {},
                    }
                ],
                "next_cursor": None,
                "limit": DEFAULT_DIRECTORY_PAGE_SIZE,
                "source": "read_model",
                "metadata": {"source_ability": "node.list"},
            },
            separators=(",", ":"),
        ).encode("utf-8")
        client = DirectoryClient(transport)

        page = client.list_devices(DeviceQuery(base_query()))

        self.assertEqual(page.limit, DEFAULT_DIRECTORY_PAGE_SIZE)
        self.assertEqual(len(page.items), 1)
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["limit"], DEFAULT_DIRECTORY_PAGE_SIZE)

    def test_list_rejects_over_max_limit_before_transport(self) -> None:
        transport = MemoryDirectoryTransport()
        client = DirectoryClient(transport)

        with self.assertRaises(SDKError):
            client.list_agents(AgentQuery(base_query(MAX_DIRECTORY_PAGE_SIZE + 1)))

        self.assertIsNone(transport.seen_request)

    def test_resolve_decodes_resolved_ref(self) -> None:
        transport = MemoryDirectoryTransport()
        transport.resolve_json = json.dumps(
            {
                "profile": "directory_identity",
                "kind": "resolved_ref",
                "answer_kind": "RESOLVE_ANSWER_KIND_FINAL_ROUTE",
                "query_name": "easynet:///r/example/device/dev-a",
                "canonical_name": "easynet:///r/example/device/dev-a",
                "owner_ura": "easynet:///r/example/device/dev-a",
                "ability_ura": "easynet:///r/example/ability/device.dev-a.agent.list",
                "route_ura": "route-ref::easynet:///r/example/ability/device.dev-a.agent.list",
                "next_hop": {"localDeviceAbility": {"dispatchName": "agent.list"}},
                "selected_route": {"reason": "ROUTE_REASON_LOCAL_DEVICE"},
                "route_candidates": [],
                "records": [],
                "negative": None,
                "release_profile": "RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL",
                "authority": {"authorityUra": "easynet:///r/example/hub"},
                "cache_policy": {"ttlMs": 0},
                "metadata": {"source": "namespace.resolve"},
            },
            separators=(",", ":"),
        ).encode("utf-8")
        client = DirectoryClient(transport)

        ref = client.resolve(
            ResolveQuery(
                base_query(),
                query_name="easynet:///r/example/device/dev-a",
                ability_name="agent.list",
                qtype="route",
            )
        )

        self.assertEqual(ref.answer_kind, "RESOLVE_ANSWER_KIND_FINAL_ROUTE")
        self.assertEqual(
            ref.ability_ura,
            "easynet:///r/example/ability/device.dev-a.agent.list",
        )
        assert transport.seen_request is not None
        self.assertEqual(
            transport.seen_request["query_name"],
            "easynet:///r/example/device/dev-a",
        )

    def test_directory_projection_helpers_delegate_to_transport(self) -> None:
        transport = MemoryDirectoryTransport()
        transport.devices_json = page_json("device_page", "device", "device")
        transport.agents_json = page_json("agent_page", "agent", "agent")
        transport.abilities_json = page_json("ability_page", "ability", "ability")
        transport.resolve_json = RESOLVED_REF_JSON
        client = DirectoryClient(transport)

        device_page = client.project_device_page(b'{"raw":"device-page"}')

        self.assertEqual(device_page.kind, "device_page")
        self.assertEqual(device_page.item_kind, "device")
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["raw"], "device-page")

        agent_page = client.project_agent_page(b'{"raw":"agent-page"}')
        self.assertEqual(agent_page.kind, "agent_page")
        self.assertEqual(agent_page.item_kind, "agent")

        ability_page = client.project_ability_page(b'{"raw":"ability-page"}')
        self.assertEqual(ability_page.kind, "ability_page")
        self.assertEqual(ability_page.item_kind, "ability")

        resolved = client.project_resolved_ref(b'{"raw":"resolve-answer"}')
        self.assertEqual(
            resolved.ability_ura,
            "easynet:///r/example/ability/device.dev-a.agent.list",
        )
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["raw"], "resolve-answer")

    def test_list_abilities_rejects_wrong_page_kind(self) -> None:
        transport = MemoryDirectoryTransport()
        transport.abilities_json = (
            b'{"profile":"directory_identity","kind":"device_page",'
            b'"item_kind":"device","items":[],"next_cursor":null,'
            b'"limit":2,"source":"read_model","metadata":{}}'
        )
        client = DirectoryClient(transport)

        with self.assertRaises(SDKError):
            client.list_abilities(AbilityQuery(base_query(2), scope="local"))

    def test_close_delegates_once_and_fails_closed(self) -> None:
        transport = MemoryDirectoryTransport()
        transport.devices_json = (
            b'{"profile":"directory_identity","kind":"device_page",'
            b'"item_kind":"device","items":[],"next_cursor":null,'
            b'"limit":50,"source":"read_model","metadata":{}}'
        )
        client = DirectoryClient(transport)

        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            client.list_devices(DeviceQuery(base_query()))
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(transport.seen_request)


def invocation_json(descriptor_ref: str) -> bytes:
    return json.dumps(
        {
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "descriptor_ref": descriptor_ref,
            "subject_ura": "easynet:///r/example/device/dev-a",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
            "args": {},
            "content_type": "application/json",
            "metadata": {"carrier_owner": "daemon_sdk"},
        },
        separators=(",", ":"),
    ).encode("utf-8")


def page_json(kind: str, item_kind: str, item_key: str) -> bytes:
    return json.dumps(
        {
            "profile": "directory_identity",
            "kind": kind,
            "item_kind": item_kind,
            "items": [{item_key + "_ura": f"easynet:///r/example/{item_key}/one"}],
            "next_cursor": None,
            "limit": 1,
            "source": "read_model",
            "metadata": {"source": "directory.project"},
        },
        separators=(",", ":"),
    ).encode("utf-8")


RESOLVED_REF_JSON = b"""
{
  "profile": "directory_identity",
  "kind": "resolved_ref",
  "answer_kind": "RESOLVE_ANSWER_KIND_FINAL_ROUTE",
  "query_name": "easynet:///r/example/device/dev-a",
  "canonical_name": "easynet:///r/example/device/dev-a",
  "owner_ura": "easynet:///r/example/device/dev-a",
  "ability_ura": "easynet:///r/example/ability/device.dev-a.agent.list",
  "route_ura": "route-ref::easynet:///r/example/ability/device.dev-a.agent.list",
  "next_hop": {"localDeviceAbility": {"dispatchName": "agent.list"}},
  "selected_route": {"reason": "ROUTE_REASON_LOCAL_DEVICE"},
  "route_candidates": [],
  "records": [],
  "negative": null,
  "release_profile": "RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL",
  "authority": {"authorityUra": "easynet:///r/example/hub"},
  "cache_policy": {"ttlMs": 0},
  "metadata": {"source": "namespace.resolve"}
}
"""

DIRECTORY_SUBSCRIPTION_INVOCATION_JSON = b"""
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.directory.subscribe@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"stream": "directory", "item_kind": "ability"},
  "content_type": "application/json",
  "metadata": {"request_id": "directory-subscribe", "profile": "directory_identity", "system_ability": "directory.subscribe", "carrier_owner": "daemon_sdk"}
}
"""

DIRECTORY_SUBSCRIPTION_JSON = b"""
{
  "profile": "directory_identity",
  "kind": "directory_subscription",
  "stream": "directory",
  "state": "Live",
  "cursor": {"stream": "directory", "sequence": 3, "token": "directory:3"},
  "resume_token": "directory:3",
  "drop_count": 0,
  "events": [
    {
      "profile": "directory_identity",
      "stream": "directory",
      "kind": "snapshot_start",
      "event_id": "evt-1",
      "phase": "snapshot_start",
      "cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
      "resume_token": "directory:1",
      "terminal": false,
      "metadata": {"source": "directory.subscribe"}
    },
    {
      "profile": "directory_identity",
      "stream": "directory",
      "kind": "snapshot_complete",
      "event_id": "evt-2",
      "phase": "snapshot_complete",
      "cursor": {"stream": "directory", "sequence": 2, "token": "directory:2"},
      "resume_token": "directory:2",
      "terminal": false,
      "metadata": {"source": "directory.subscribe"}
    },
    {
      "profile": "directory_identity",
      "stream": "directory",
      "kind": "upsert",
      "event_id": "evt-3",
      "phase": "live",
      "item_kind": "ability",
      "item": {"ability_ura": "easynet:///r/example/ability/device.dev-a.agent.list"},
      "cursor": {"stream": "directory", "sequence": 3, "token": "directory:3"},
      "resume_token": "directory:3",
      "terminal": false,
      "metadata": {"source": "directory.subscribe"}
    }
  ],
  "metadata": {"source": "directory.subscribe"}
}
"""

DIRECTORY_SUBSCRIPTION_LIVE_BEFORE_SNAPSHOT_JSON = b"""
{
  "profile": "directory_identity",
  "kind": "directory_subscription",
  "stream": "directory",
  "state": "Live",
  "cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
  "resume_token": "directory:1",
  "drop_count": 0,
  "events": [{
    "profile": "directory_identity",
    "stream": "directory",
    "kind": "upsert",
    "event_id": "evt-1",
    "phase": "live",
    "cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
    "resume_token": "directory:1",
    "terminal": false,
    "metadata": {}
  }],
  "metadata": {}
}
"""

DIRECTORY_SUBSCRIPTION_DUPLICATE_EVENT_JSON = b"""
{
  "profile": "directory_identity",
  "kind": "directory_subscription",
  "stream": "directory",
  "state": "Live",
  "cursor": {"stream": "directory", "sequence": 2, "token": "directory:2"},
  "resume_token": "directory:2",
  "drop_count": 0,
  "events": [
    {
      "profile": "directory_identity",
      "stream": "directory",
      "kind": "snapshot_complete",
      "event_id": "evt-1",
      "phase": "snapshot_complete",
      "cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
      "resume_token": "directory:1",
      "terminal": false,
      "metadata": {}
    },
    {
      "profile": "directory_identity",
      "stream": "directory",
      "kind": "upsert",
      "event_id": "evt-1",
      "phase": "live",
      "cursor": {"stream": "directory", "sequence": 2, "token": "directory:2"},
      "resume_token": "directory:2",
      "terminal": false,
      "metadata": {}
    }
  ],
  "metadata": {}
}
"""


if __name__ == "__main__":
    unittest.main()
