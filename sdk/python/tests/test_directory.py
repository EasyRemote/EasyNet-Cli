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
    ResolveQuery,
    SDKError,
)


class MemoryDirectoryTransport:
    def __init__(self) -> None:
        self.resolve_json = b"{}"
        self.devices_json = b"{}"
        self.agents_json = b"{}"
        self.abilities_json = b"{}"
        self.seen_request: dict[str, object] | None = None

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


if __name__ == "__main__":
    unittest.main()
