import json
import unittest

from easynet_sdk import EasyRemoteAdminAdapter, ErrorCode, SDKError, is_code
from easynet_sdk.admin import (
    AdminAgentListRequest,
    AdminAgentRefreshRequest,
    AdminAgentStartRequest,
    AdminAgentStopRequest,
    AdminCarrierBase,
    AdminClient,
    AdminGatewayStatusRequest,
    AdminJoinHubRequest,
    AdminLeaveHubRequest,
    AdminSessionListRequest,
    CreateDeviceSessionRequest,
    CreatePairingRequest,
    DeleteDeviceSessionRequest,
    PairingPreflightRequest,
    RevokeDeviceRequest,
    ValidatePairingRequest,
    VerifyDeviceCredentialRequest,
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

ADMIN_JOIN_RESULT = b"""{
  "profile": "admin_gateway",
  "kind": "hub_membership_result",
  "operation": "hub.join",
  "state": "ok",
  "device_ura": "easynet:///r/example/device/dev-a",
  "ack": true,
  "metadata": {"profile": "admin_gateway", "source": "hub.join"}
}"""

ADMIN_LEAVE_RESULT = b"""{
  "profile": "admin_gateway",
  "kind": "hub_membership_result",
  "operation": "hub.leave",
  "state": "ok",
  "device_ura": "easynet:///r/example/device/dev-a",
  "ack": true,
  "metadata": {"profile": "admin_gateway", "source": "hub.leave"}
}"""

ADMIN_PAIRING_PREFLIGHT = b"""{
  "profile": "admin_gateway",
  "kind": "pairing_preflight",
  "state": "requires_pairing",
  "hub_ura": "easynet:///r/example/hub/main",
  "device_ura": "easynet:///r/example/device/dev-a",
  "pairing_required": true,
  "trust_ready": false,
  "scopes": ["invoke", "events"],
  "metadata": {"profile": "admin_gateway", "source": "pairing.preflight"}
}"""

ADMIN_PAIRING_TOKEN = b"""{
  "profile": "admin_gateway",
  "kind": "pairing_token",
  "token_id": "pair-token-1",
  "token": "pair-token-value",
  "hub_ura": "easynet:///r/example/hub/main",
  "device_ura": "easynet:///r/example/device/dev-a",
  "state": "issued",
  "expires_unix_ms": 1893456000000,
  "scopes": ["invoke", "events"],
  "metadata": {"profile": "admin_gateway", "source": "pairing.create"}
}"""

ADMIN_DEVICE_CREDENTIAL = b"""{
  "profile": "admin_gateway",
  "kind": "device_credential",
  "credential_id": "cred-dev-a",
  "device_ura": "easynet:///r/example/device/dev-a",
  "hub_ura": "easynet:///r/example/hub/main",
  "state": "active",
  "issued_unix_ms": 1767225600000,
  "expires_unix_ms": 1893456000000,
  "scopes": ["invoke", "events"],
  "metadata": {"profile": "admin_gateway", "source": "pairing.validate"}
}"""

ADMIN_CREDENTIAL_VERIFICATION = b"""{
  "profile": "admin_gateway",
  "kind": "device_credential_verification",
  "verified": true,
  "credential_id": "cred-dev-a",
  "device_ura": "easynet:///r/example/device/dev-a",
  "hub_ura": "easynet:///r/example/hub/main",
  "method": "daemon-trust-store",
  "metadata": {"profile": "admin_gateway", "source": "credential.verify"}
}"""

ADMIN_DEVICE_SESSION = b"""{
  "profile": "admin_gateway",
  "kind": "device_session",
  "session_id": "dev-session-1",
  "device_ura": "easynet:///r/example/device/dev-a",
  "hub_ura": "easynet:///r/example/hub/main",
  "state": "active",
  "session_kind": "remote_desktop",
  "created_unix_ms": 1767225600000,
  "expires_unix_ms": 1893456000000,
  "metadata": {"profile": "admin_gateway", "source": "session.create"}
}"""

ADMIN_DEVICE_SESSION_PAGE = b"""{
  "profile": "admin_gateway",
  "kind": "device_sessions",
  "state": "ok",
  "items": [{
    "profile": "admin_gateway",
    "kind": "device_session",
    "session_id": "dev-session-1",
    "device_ura": "easynet:///r/example/device/dev-a",
    "hub_ura": "easynet:///r/example/hub/main",
    "state": "active",
    "session_kind": "remote_desktop",
    "created_unix_ms": 1767225600000,
    "expires_unix_ms": 1893456000000,
    "metadata": {"profile": "admin_gateway", "source": "session.list"}
  }],
  "next_cursor": null,
  "metadata": {"profile": "admin_gateway", "source": "session.list"}
}"""

ADMIN_REVOKE_DEVICE_RESULT = b"""{
  "profile": "admin_gateway",
  "kind": "device_admin_result",
  "operation": "device.revoke",
  "state": "revoked",
  "device_ura": "easynet:///r/example/device/dev-a",
  "ack": true,
  "metadata": {"profile": "admin_gateway", "source": "device.revoke"}
}"""

ADMIN_DELETE_SESSION_RESULT = b"""{
  "profile": "admin_gateway",
  "kind": "device_admin_result",
  "operation": "session.delete",
  "state": "deleted",
  "device_ura": "easynet:///r/example/device/dev-a",
  "ack": true,
  "metadata": {"profile": "admin_gateway", "source": "session.delete"}
}"""


class MemoryAdminTransport:
    def __init__(self) -> None:
        self.seen: dict[str, dict[str, object]] = {}
        self.close_calls = 0

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
        return ADMIN_DEVICE_SESSION_PAGE

    def join_hub(self, request_json: bytes) -> bytes:
        self._remember("join_hub", request_json)
        return ADMIN_JOIN_RESULT

    def leave_hub(self, request_json: bytes) -> bytes:
        self._remember("leave_hub", request_json)
        return ADMIN_LEAVE_RESULT

    def pairing_preflight(self, request_json: bytes) -> bytes:
        self._remember("pairing_preflight", request_json)
        return ADMIN_PAIRING_PREFLIGHT

    def validate_pairing(self, request_json: bytes) -> bytes:
        self._remember("validate_pairing", request_json)
        return ADMIN_DEVICE_CREDENTIAL

    def verify_device_credential(self, request_json: bytes) -> bytes:
        self._remember("verify_device_credential", request_json)
        return ADMIN_CREDENTIAL_VERIFICATION

    def create_pairing(self, request_json: bytes) -> bytes:
        self._remember("create_pairing", request_json)
        return ADMIN_PAIRING_TOKEN

    def revoke_device(self, request_json: bytes) -> bytes:
        self._remember("revoke_device", request_json)
        return ADMIN_REVOKE_DEVICE_RESULT

    def create_device_session(self, request_json: bytes) -> bytes:
        self._remember("create_device_session", request_json)
        return ADMIN_DEVICE_SESSION

    def delete_device_session(self, request_json: bytes) -> bytes:
        self._remember("delete_device_session", request_json)
        return ADMIN_DELETE_SESSION_RESULT

    def close(self) -> None:
        self.close_calls += 1


class _EasyRemoteInvocation:
    def __init__(self, response: dict[str, object]) -> None:
        self._response = response

    def result(self) -> dict[str, object]:
        return self._response


class _EasyRemoteClient:
    def __init__(self, *responses: dict[str, object]) -> None:
        self.responses = list(responses)
        self.invocations: list[dict[str, object]] = []

    def invoke(self, ability: str, **kwargs: object) -> _EasyRemoteInvocation:
        self.invocations.append({"ability": ability, "args": dict(kwargs)})
        return _EasyRemoteInvocation(self.responses.pop(0))


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
    def test_easyremote_adapter_controls_hosted_agents(self) -> None:
        client = _EasyRemoteClient(
            {"root_path": "/tmp/agent", "model": "gpt-5", "replaced_prior": True},
            {
                "agents": [
                    {
                        "name": "codex",
                        "runtime": "codex",
                        "model": "gpt-5",
                        "root_path": "/tmp/agent",
                        "root_exists": True,
                        "timeout_secs": 600,
                    }
                ]
            },
            {"agents_scanned": 1},
        )
        adapter = EasyRemoteAdminAdapter(client)

        started = adapter.start_agent(
            "codex",
            kind="codex",
            model="gpt-5",
            label="primary",
            command="codex",
            args=("--ask",),
        )
        records = adapter.list_agents()
        refreshed = adapter.refresh_agents("codex")

        self.assertEqual(started.name, "codex")
        self.assertEqual(started.runtime, "codex")
        self.assertTrue(started.replaced_prior)
        self.assertEqual(records[0].root_path, "/tmp/agent")
        self.assertEqual(records[0].timeout_secs, 600)
        self.assertEqual(refreshed["agents_scanned"], 1)
        self.assertEqual(client.invocations[0]["ability"], "agent.start")
        self.assertEqual(
            client.invocations[0]["args"],
            {
                "name": "codex",
                "agent_type": "codex",
                "model": "gpt-5",
                "model_present": True,
                "label": "primary",
                "command": "codex",
                "command_args": ["--ask"],
                "materialize_directory": True,
                "update_existing_spec": False,
                "project_workspace": True,
            },
        )
        self.assertEqual(client.invocations[1], {"ability": "agent.list", "args": {}})
        self.assertEqual(
            client.invocations[2],
            {"ability": "agent.refresh", "args": {"name": "codex"}},
        )

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

    def test_trust_and_device_session_lifecycle(self) -> None:
        transport = MemoryAdminTransport()
        client = AdminClient(transport)

        join = client.join_hub(
            AdminJoinHubRequest(
                admin_base(),
                hub_ura="easynet:///r/example/hub/main",
                device_ura="easynet:///r/example/device/dev-a",
            )
        )
        self.assertEqual(join.operation, "hub.join")
        self.assertEqual(
            transport.seen["join_hub"]["hub_ura"], "easynet:///r/example/hub/main"
        )

        preflight = client.pairing_preflight(
            PairingPreflightRequest(
                admin_base(),
                hub_ura="easynet:///r/example/hub/main",
                device_ura="easynet:///r/example/device/dev-a",
                requested_scopes=("invoke", "events"),
            )
        )
        self.assertTrue(preflight.pairing_required)
        self.assertFalse(preflight.trust_ready)

        token = client.create_pairing(
            CreatePairingRequest(
                admin_base(),
                hub_ura="easynet:///r/example/hub/main",
                device_ura="easynet:///r/example/device/dev-a",
                expires_unix_ms=1893456000000,
                scopes=("invoke", "events"),
            )
        )
        self.assertEqual(token.token_id, "pair-token-1")

        credential = client.validate_pairing(
            ValidatePairingRequest(
                admin_base(),
                token="pair-token-value",
                device_ura="easynet:///r/example/device/dev-a",
            )
        )
        self.assertEqual(credential.credential_id, "cred-dev-a")
        self.assertEqual(credential.state, "active")

        verification = client.verify_device_credential(
            VerifyDeviceCredentialRequest(
                admin_base(),
                credential_id="cred-dev-a",
                device_ura="easynet:///r/example/device/dev-a",
                hub_ura="easynet:///r/example/hub/main",
            )
        )
        self.assertTrue(verification.verified)
        self.assertEqual(verification.method, "daemon-trust-store")

        session = client.create_device_session(
            CreateDeviceSessionRequest(
                admin_base(),
                device_ura="easynet:///r/example/device/dev-a",
                hub_ura="easynet:///r/example/hub/main",
                session_kind="remote_desktop",
                expires_unix_ms=1893456000000,
            )
        )
        self.assertEqual(session.session_id, "dev-session-1")
        self.assertEqual(session.session_kind, "remote_desktop")

        page = client.list_device_sessions(AdminSessionListRequest(admin_base()))
        self.assertEqual(len(page.items), 1)
        self.assertEqual(page.items[0].session_id, "dev-session-1")

        leave = client.leave_hub(
            AdminLeaveHubRequest(
                admin_base(),
                hub_ura="easynet:///r/example/hub/main",
                reason="rotation",
            )
        )
        self.assertEqual(leave.operation, "hub.leave")

        revoked = client.revoke_device(
            RevokeDeviceRequest(
                admin_base(),
                device_ura="easynet:///r/example/device/dev-a",
                reason="compromised",
            )
        )
        self.assertEqual(revoked.operation, "device.revoke")

        deleted = client.delete_device_session(
            DeleteDeviceSessionRequest(
                admin_base(), session_id="dev-session-1", reason="done"
            )
        )
        self.assertEqual(deleted.operation, "session.delete")

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
        with self.assertRaises(Exception):
            client.create_pairing(
                CreatePairingRequest(
                    admin_base(),
                    hub_ura="not-a-hub",
                    device_ura="easynet:///r/example/device/dev-a",
                    expires_unix_ms=1,
                )
            )
        with self.assertRaises(Exception):
            client.validate_pairing(
                ValidatePairingRequest(
                    admin_base(),
                    token="../pairing",
                    device_ura="easynet:///r/example/device/dev-a",
                )
            )
        with self.assertRaises(Exception):
            client.delete_device_session(
                DeleteDeviceSessionRequest(admin_base(), session_id="browser-session-1")
            )

    def test_close_delegates_once_and_fails_closed(self) -> None:
        transport = MemoryAdminTransport()
        client = AdminClient(transport)

        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            client.build_agent_list_invocation(AdminAgentListRequest(admin_base()))
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(transport.seen, {})


if __name__ == "__main__":
    unittest.main()
