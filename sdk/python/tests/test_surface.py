import json
import unittest

from easynet_sdk import ErrorCode, SDKError, is_code
from easynet_sdk.surface import (
    MAX_SURFACE_PAGE_SIZE,
    SurfaceCarrierBase,
    SurfaceClient,
    SurfaceCreatePageRequest,
    SurfaceDeletePageRequest,
    SurfaceHealthRequest,
    SurfaceListPagesRequest,
    SurfaceManifestRequest,
    SurfaceStatusRequest,
)


SURFACE_LIST_INVOCATION = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/agent/alice.pages",
  "descriptor_ref": "easynet:///r/example/ability/alice.pages.pages.list@1.0.0",
  "subject_ura": "easynet:///r/example/agent/alice.pages",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {},
  "content_type": "application/json",
  "metadata": {"request_id": "surface-list-1", "profile": "surface", "system_ability": "pages.list", "carrier_owner": "daemon_sdk"}
}"""

SURFACE_CREATE_INVOCATION = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/agent/alice.pages",
  "descriptor_ref": "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0",
  "subject_ura": "easynet:///r/example/agent/alice.pages",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"project_id": "docs", "folder": "/tmp/easynet-pages-docs", "visibility": "public"},
  "content_type": "application/json",
  "metadata": {"request_id": "surface-create-1", "profile": "surface", "system_ability": "pages.publish", "carrier_owner": "daemon_sdk"}
}"""

SURFACE_DELETE_INVOCATION = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/agent/alice.pages",
  "descriptor_ref": "easynet:///r/example/ability/alice.pages.pages.unpublish@1.0.0",
  "subject_ura": "easynet:///r/example/agent/alice.pages",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"project_id": "docs"},
  "content_type": "application/json",
  "metadata": {"request_id": "surface-delete-1", "profile": "surface", "system_ability": "pages.unpublish", "carrier_owner": "daemon_sdk"}
}"""

SURFACE_MANIFEST_INVOCATION = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/agent/alice.pages",
  "descriptor_ref": "easynet:///r/example/ability/alice.pages.pages.get@1.0.0",
  "subject_ura": "easynet:///r/example/agent/alice.pages",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"project_id": "docs"},
  "content_type": "application/json",
  "metadata": {"request_id": "surface-manifest-1", "profile": "surface", "system_ability": "pages.get", "carrier_owner": "daemon_sdk"}
}"""

SURFACE_HEALTH_INVOCATION = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/agent/alice.pages",
  "descriptor_ref": "easynet:///r/example/ability/alice.pages.pages.health@1.0.0",
  "subject_ura": "easynet:///r/example/agent/alice.pages",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"surface_ref": "easynet:///r/example/resource/alice.docs"},
  "content_type": "application/json",
  "metadata": {"request_id": "surface-health-1", "profile": "surface", "system_ability": "pages.health", "carrier_owner": "daemon_sdk"}
}"""

SURFACE_PAGE_RECORD = b"""{
  "profile": "surface",
  "kind": "page_record",
  "page_id": "docs",
  "owner_ura": "easynet:///r/example/agent/alice.pages",
  "surface_ref": "easynet:///r/example/resource/alice.docs",
  "public_ref": "https://example/web/alice/docs/",
  "status": "published",
  "metadata": {"profile": "surface", "source_ability": "pages.get", "user": "alice", "project_id": "docs", "visibility": "public"}
}"""

SURFACE_PAGE_PAGE = (
    b"""{
  "profile": "surface",
  "kind": "surface_page_page",
  "item_kind": "page_record",
  "items": ["""
    + SURFACE_PAGE_RECORD
    + b"""],
  "next_cursor": null,
  "limit": 50,
  "source": "pages_read_model",
  "metadata": {"profile": "surface", "source_ability": "pages.list", "page_size_default": 50, "page_size_max": 500, "total_available": 1}
}"""
)

SURFACE_PUBLIC_PAGE_REF = b"""{
  "profile": "surface",
  "kind": "public_page_ref",
  "page_id": "docs",
  "owner_ura": "easynet:///r/example/agent/alice.pages",
  "surface_ref": "easynet:///r/example/resource/alice.docs",
  "public_ref": "https://example/web/alice/docs/",
  "route_kind": "hub_web",
  "metadata": {"profile": "surface", "source_ability": "pages.get"}
}"""

SURFACE_MANIFEST = (
    b"""{
  "profile": "surface",
  "kind": "surface_manifest",
  "page_id": "docs",
  "owner_ura": "easynet:///r/example/agent/alice.pages",
  "surface_ref": "easynet:///r/example/resource/alice.docs",
  "public_ref": "https://example/web/alice/docs/",
  "page": """
    + SURFACE_PAGE_RECORD
    + b""",
  "entrypoint": {"kind": "public_page_ref", "href": "https://example/web/alice/docs/"},
  "metadata": {"profile": "surface", "source_ability": "pages.get"}
}"""
)

SURFACE_MUTATION_RESULT = b"""{
  "profile": "surface",
  "kind": "surface_mutation_result",
  "operation": "delete",
  "page_id": "docs",
  "removed": true,
  "state": "deleted",
  "metadata": {"profile": "surface", "source_ability": "pages.unpublish"}
}"""

SURFACE_HEALTH = b"""{
  "profile": "surface",
  "kind": "surface_health",
  "state": "ready",
  "ready": true,
  "owner_ura": "easynet:///r/example/agent/alice.pages",
  "surface_ref": "easynet:///r/example/resource/alice.docs",
  "descriptor_ref": "easynet:///r/example/ability/alice.pages.pages.health@1.0.0",
  "descriptor_version": "1.0.0",
  "page_count": 1,
  "checks": [
    {"name": "manifest", "state": "ready", "ready": true, "message": null, "latency_ms": 3, "metadata": {"source": "pages.get"}},
    {"name": "public_ref", "state": "ready", "ready": true, "message": null, "latency_ms": 1, "metadata": {"route_kind": "hub_web"}}
  ],
  "metadata": {"profile": "surface", "source_ability": "pages.health", "rendering_owner": "backend"}
}"""


class MemorySurfaceTransport:
    def __init__(self) -> None:
        self.seen: dict[str, dict[str, object]] = {}
        self.close_calls = 0

    def _remember(self, name: str, request_json: bytes) -> None:
        self.seen[name] = json.loads(request_json.decode("utf-8"))

    def build_list_pages_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_list", request_json)
        return SURFACE_LIST_INVOCATION

    def build_create_page_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_create", request_json)
        return SURFACE_CREATE_INVOCATION

    def build_delete_page_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_delete", request_json)
        return SURFACE_DELETE_INVOCATION

    def build_manifest_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_manifest", request_json)
        return SURFACE_MANIFEST_INVOCATION

    def build_health_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_health", request_json)
        return SURFACE_HEALTH_INVOCATION

    def list_pages(self, request_json: bytes) -> bytes:
        self._remember("list_pages", request_json)
        return SURFACE_PAGE_PAGE

    def create_page(self, request_json: bytes) -> bytes:
        self._remember("create_page", request_json)
        return SURFACE_PAGE_RECORD

    def delete_page(self, request_json: bytes) -> bytes:
        self._remember("delete_page", request_json)
        return SURFACE_MUTATION_RESULT

    def surface_manifest(self, request_json: bytes) -> bytes:
        self._remember("surface_manifest", request_json)
        return SURFACE_MANIFEST

    def public_page_ref(self, request_json: bytes) -> bytes:
        self._remember("public_page_ref", request_json)
        return SURFACE_PUBLIC_PAGE_REF

    def surface_health(self, request_json: bytes) -> bytes:
        self._remember("surface_health", request_json)
        return SURFACE_HEALTH

    def close(self) -> None:
        self.close_calls += 1


def surface_base() -> SurfaceCarrierBase:
    return SurfaceCarrierBase(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura="easynet:///r/example/agent/alice.pages",
        subject_ura="easynet:///r/example/agent/alice.pages",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        metadata={"request_id": "surface-list-1"},
    )


class SurfaceClientTests(unittest.TestCase):
    def test_builds_page_invocations(self) -> None:
        transport = MemorySurfaceTransport()
        client = SurfaceClient(transport)

        list_draft = client.build_list_pages_invocation(
            SurfaceListPagesRequest(surface_base(), limit=50)
        )
        self.assertEqual(
            list_draft.descriptor_ref,
            "easynet:///r/example/ability/alice.pages.pages.list@1.0.0",
        )

        create_draft = client.build_create_page_invocation(
            SurfaceCreatePageRequest(
                surface_base(),
                project_id="docs",
                folder="/tmp/easynet-pages-docs",
                visibility="public",
            )
        )
        self.assertEqual(
            create_draft.descriptor_ref,
            "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0",
        )
        self.assertEqual(transport.seen["build_create"]["project_id"], "docs")

        delete_draft = client.build_delete_page_invocation(
            SurfaceDeletePageRequest(surface_base(), project_id="docs")
        )
        self.assertEqual(
            delete_draft.descriptor_ref,
            "easynet:///r/example/ability/alice.pages.pages.unpublish@1.0.0",
        )

        manifest_draft = client.build_manifest_invocation(
            SurfaceManifestRequest(surface_base(), project_id="docs")
        )
        self.assertEqual(
            manifest_draft.descriptor_ref,
            "easynet:///r/example/ability/alice.pages.pages.get@1.0.0",
        )

        health_draft = client.build_health_invocation(
            SurfaceHealthRequest(
                surface_base(), surface_ref="easynet:///r/example/resource/alice.docs"
            )
        )
        self.assertEqual(
            health_draft.descriptor_ref,
            "easynet:///r/example/ability/alice.pages.pages.health@1.0.0",
        )
        self.assertEqual(
            transport.seen["build_health"]["surface_ref"],
            "easynet:///r/example/resource/alice.docs",
        )

    def test_projects_pages_manifest_ref_and_mutation(self) -> None:
        client = SurfaceClient(MemorySurfaceTransport())

        page = client.list_pages(SurfaceListPagesRequest(surface_base(), limit=50))
        self.assertEqual(len(page.items), 1)
        self.assertEqual(page.items[0].page_id, "docs")
        self.assertEqual(page.source, "pages_read_model")

        record = client.create_page(
            SurfaceCreatePageRequest(
                surface_base(),
                project_id="docs",
                folder="/tmp/easynet-pages-docs",
                visibility="public",
            )
        )
        self.assertEqual(record.page_id, "docs")
        self.assertIsNotNone(record.public_ref)

        manifest = client.surface_manifest(
            SurfaceManifestRequest(surface_base(), project_id="docs")
        )
        self.assertEqual(manifest.kind, "surface_manifest")
        self.assertEqual(manifest.page.page_id, "docs")

        ref = client.public_page_ref(record)
        self.assertEqual(ref.route_kind, "hub_web")
        self.assertTrue(ref.public_ref)

        result = client.delete_page(
            SurfaceDeletePageRequest(surface_base(), project_id="docs")
        )
        self.assertTrue(result.removed)
        self.assertEqual(result.state, "deleted")

        health = client.surface_health(
            SurfaceHealthRequest(surface_base(), project_id="docs")
        )
        self.assertTrue(health.ready)
        self.assertEqual(health.page_count, 1)
        self.assertEqual(len(health.checks), 2)
        self.assertEqual(health.checks[0].name, "manifest")

        status = client.surface_status(
            SurfaceStatusRequest(surface_base(), project_id="docs")
        )
        self.assertEqual(status.surface_ref, health.surface_ref)

    def test_rejects_invalid_requests(self) -> None:
        client = SurfaceClient(MemorySurfaceTransport())

        with self.assertRaises(Exception):
            client.build_create_page_invocation(
                SurfaceCreatePageRequest(
                    SurfaceCarrierBase("", "", "", "", "", {}),
                    project_id="docs",
                    folder="/tmp/pages",
                )
            )
        with self.assertRaises(Exception):
            client.build_create_page_invocation(
                SurfaceCreatePageRequest(
                    surface_base(), project_id="../docs", folder="/tmp/pages"
                )
            )
        with self.assertRaises(Exception):
            client.build_create_page_invocation(
                SurfaceCreatePageRequest(
                    surface_base(), project_id="docs", folder="relative"
                )
            )
        with self.assertRaises(Exception):
            client.list_pages(
                SurfaceListPagesRequest(
                    surface_base(), limit=MAX_SURFACE_PAGE_SIZE + 1
                )
            )
        with self.assertRaises(Exception):
            client.build_health_invocation(
                SurfaceHealthRequest(
                    surface_base(), surface_ref="https://example/web/alice/docs/"
                )
            )

    def test_close_delegates_once_and_fails_closed(self) -> None:
        transport = MemorySurfaceTransport()
        client = SurfaceClient(transport)

        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            client.build_list_pages_invocation(
                SurfaceListPagesRequest(surface_base(), limit=50)
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(transport.seen, {})


if __name__ == "__main__":
    unittest.main()
