import json
import unittest

from easynet_sdk.admin import (
    AdminAgentListRequest,
    AdminAgentRefreshRequest,
    AdminAgentStartRequest,
    AdminAgentStopRequest,
    AdminCarrierBase,
    AdminClient,
    AdminGatewayStatusRequest,
    AdminSessionListRequest,
)


ADMIN_AGENT_LIST_INVOCATION = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-agent-list-1",
    "profile": "admin_gateway",
    "system_ability": "agent.list",
    "carrier_owner": "daemon_sdk"
  }
}"""

ADMIN_AGENT_START_INVOCATION = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"name": "codex", "agent_type": "codex", "model": "gpt-5", "label": "primary"},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-agent-start-1",
    "profile": "admin_gateway",
    "system_ability": "agent.start",
    "carrier_owner": "daemon_sdk"
  }
}"""

ADMIN_AGENT_STOP_INVOCATION = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"name": "codex"},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-agent-stop-1",
    "profile": "admin_gateway",
    "system_ability": "agent.stop",
    "carrier_owner": "daemon_sdk"
  }
}"""

ADMIN_AGENT_REFRESH_INVOCATION = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"name": "codex"},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-agent-refresh-1",
    "profile": "admin_gateway",
    "system_ability": "agent.refresh",
    "carrier_owner": "daemon_sdk"
  }
}"""

ADMIN_SESSION_LIST_INVOCATION = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.session.list@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"include_terminated": false},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-session-list-1",
    "profile": "admin_gateway",
    "system_ability": "session.list",
    "carrier_owner": "daemon_sdk"
  }
}"""

ADMIN_GATEWAY_STATUS = b"""{
  "profile": "admin_gateway",
  "gateway_id": "device:example:dev-a",
  "ready": true,
  "state": "ready",
  "process_live": true,
  "control_ready": true,
  "runtime_ready": true,
  "directory_ready": true,
  "trust_ready": true,
  "public_listener_ready": false,
  "listeners": [
    {"kind": "control", "endpoint": "/tmp/easynet-control.sock", "ready": true, "public": false},
    {"kind": "invocation", "endpoint": "/tmp/easynet-daemon.sock", "ready": true, "public": false}
  ],
  "identity": {"mode": "device", "realm": "example", "node_id": "dev-a"},
  "metadata": {
    "profile": "admin_gateway",
    "source": "daemon_lifecycle_status",
    "lifecycle_state": "running",
    "requires_public_listener": false
  }
}"""

ADMIN_AGENT_RECORDS = b"""{
  "profile": "admin_gateway",
  "kind": "agent_records",
  "state": "ok",
  "items": [{
    "name": "codex",
    "agent_ura": "easynet:///r/example/agent/alice.codex",
    "owner_ura": "easynet:///r/example/user/alice",
    "device_ura": null,
    "state": "registered",
    "runtime": "codex",
    "model": "gpt-5",
    "label": "primary",
    "abilities": [],
    "metadata": {
      "profile": "admin_gateway",
      "source": "agent.list",
      "root_path": "/tmp/easynet/agents/codex",
      "root_exists": true,
      "timeout_secs": 600
    }
  }],
  "next_cursor": null,
  "metadata": {"profile": "admin_gateway", "source": "agent.list", "count": 1}
}"""

ADMIN_LIFECYCLE_RESULT = b"""{
  "profile": "admin_gateway",
  "kind": "agent_lifecycle_result",
  "operation": "agent.start",
  "state": "ok",
  "agent_ura": "easynet:///r/example/agent/alice.codex",
  "ack": null,
  "runtime_not_ready": false,
  "runtime_catalog_not_ready": false,
  "metadata": {
    "profile": "admin_gateway",
    "source": "agent_lifecycle",
    "runtime_registered": 3,
    "runtime_failed": 0,
    "runtime_removed": 0,
    "raw_result": {
      "agent_ura": "easynet:///r/example/agent/alice.codex",
      "replaced_prior": false,
      "runtime_registered": 3,
      "runtime_failed": 0,
      "runtime_removed": 0,
      "runtime_not_ready": false,
      "runtime_catalog_not_ready": false
    }
  }
}"""


class MemoryAdminTransport:
    def __init__(self) -> None:
        self.seen: dict[str, dict[str, object]] = {}

    def _remember(self, name: str, request_json: bytes) -> None:
        self.seen[name] = json.loads(request_json.decode("utf-8"))

    def build_agent_list_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_agent_list", request_json)
        return ADMIN_AGENT_LIST_INVOCATION

    def build_agent_start_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_agent_start", request_json)
        return ADMIN_AGENT_START_INVOCATION

    def build_agent_stop_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_agent_stop", request_json)
        return ADMIN_AGENT_STOP_INVOCATION

    def build_agent_refresh_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_agent_refresh", request_json)
        return ADMIN_AGENT_REFRESH_INVOCATION

    def build_session_list_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_session_list", request_json)
        return ADMIN_SESSION_LIST_INVOCATION

    def gateway_status(self, request_json: bytes) -> bytes:
        self._remember("gateway_status", request_json)
        return ADMIN_GATEWAY_STATUS

    def list_agents(self, request_json: bytes) -> bytes:
        self._remember("list_agents", request_json)
        return ADMIN_AGENT_RECORDS

    def agent_start(self, request_json: bytes) -> bytes:
        self._remember("agent_start", request_json)
        return ADMIN_LIFECYCLE_RESULT

    def agent_stop(self, request_json: bytes) -> bytes:
        self._remember("agent_stop", request_json)
        return ADMIN_LIFECYCLE_RESULT

    def agent_refresh(self, request_json: bytes) -> bytes:
        self._remember("agent_refresh", request_json)
        return ADMIN_LIFECYCLE_RESULT

    def list_device_sessions(self, request_json: bytes) -> bytes:
        self._remember("list_device_sessions", request_json)
        return b'{"profile":"admin_gateway","kind":"device_sessions","state":"ok","items":[],"next_cursor":null,"metadata":{"profile":"admin_gateway","source":"session.list"}}'


def admin_base() -> AdminCarrierBase:
    return AdminCarrierBase(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/device/dev-a",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        metadata={"request_id": "admin-agent-list-1"},
    )


class AdminClientTests(unittest.TestCase):
    def test_builds_agent_and_session_invocations(self) -> None:
        transport = MemoryAdminTransport()
        client = AdminClient(transport)

        list_draft = client.build_agent_list_invocation(AdminAgentListRequest(admin_base()))
        self.assertEqual(
            list_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0",
        )
        self.assertEqual(
            transport.seen["build_agent_list"]["caller_ura"],
            "easynet:///r/example/agent/alice.sdk",
        )

        start_draft = client.build_agent_start_invocation(
            AdminAgentStartRequest(
                admin_base(),
                name="codex",
                agent_type="codex",
                model="gpt-5",
                label="primary",
            )
        )
        self.assertEqual(
            start_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0",
        )
        self.assertEqual(transport.seen["build_agent_start"]["name"], "codex")

        stop_draft = client.build_agent_stop_invocation(
            AdminAgentStopRequest(admin_base(), name="codex")
        )
        self.assertEqual(
            stop_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0",
        )

        refresh_draft = client.build_agent_refresh_invocation(
            AdminAgentRefreshRequest(admin_base(), name="codex")
        )
        self.assertEqual(
            refresh_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0",
        )

        session_draft = client.build_session_list_invocation(
            AdminSessionListRequest(admin_base(), include_terminated=False)
        )
        self.assertEqual(
            session_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.session.list@1.0.0",
        )
        self.assertEqual(
            transport.seen["build_session_list"]["include_terminated"], False
        )

    def test_projects_gateway_agents_and_lifecycle(self) -> None:
        client = AdminClient(MemoryAdminTransport())

        status = client.gateway_status(AdminGatewayStatusRequest())
        self.assertTrue(status.control_ready)
        self.assertTrue(status.runtime_ready)
        self.assertFalse(status.public_listener_ready)
        self.assertEqual(status.metadata["source"], "daemon_lifecycle_status")

        page = client.list_agents(AdminAgentListRequest(admin_base()))
        self.assertEqual(len(page.items), 1)
        self.assertEqual(page.items[0].name, "codex")
        self.assertEqual(page.items[0].runtime, "codex")

        result = client.agent_start(
            AdminAgentStartRequest(admin_base(), name="codex", agent_type="codex")
        )
        self.assertEqual(result.operation, "agent.start")
        self.assertEqual(result.state, "ok")
        self.assertEqual(result.agent_ura, "easynet:///r/example/agent/alice.codex")

    def test_rejects_incomplete_carrier_and_system_lifecycle(self) -> None:
        client = AdminClient(MemoryAdminTransport())

        with self.assertRaises(Exception):
            client.build_agent_start_invocation(
                AdminAgentStartRequest(
                    AdminCarrierBase("", "", "", "", "", {}),
                    name="codex",
                    agent_type="codex",
                )
            )
        with self.assertRaises(Exception):
            client.build_agent_start_invocation(
                AdminAgentStartRequest(admin_base(), name="device", agent_type="codex")
            )
        with self.assertRaises(Exception):
            client.build_agent_start_invocation(
                AdminAgentStartRequest(admin_base(), name="../codex", agent_type="codex")
            )
        with self.assertRaises(Exception):
            client.build_agent_stop_invocation(
                AdminAgentStopRequest(
                    admin_base(), agent_ura="easynet:///r/example/device/dev-a"
                )
            )


if __name__ == "__main__":
    unittest.main()
