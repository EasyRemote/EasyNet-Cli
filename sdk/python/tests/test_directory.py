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
    DirectorySubscriptionCursor,
    DirectorySubscriptionRequest,
    ErrorCode,
    ResolveQuery,
    SDKError,
    is_code,
)


class MemoryDirectoryTransport:
    def __init__(self) -> None:
        self.resolve_json = b"{}"
        self.devices_json = b"{}"
        self.agents_json = b"{}"
        self.abilities_json = b"{}"
        self.subscription_invocation_json = b"{}"
        self.subscription_json = b"{}"
        self.seen_request: dict[str, object] | None = None
        self.close_calls = 0

    def build_directory_subscription_invocation(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.subscription_invocation_json

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
