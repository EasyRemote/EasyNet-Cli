import unittest
from collections.abc import Mapping
from pathlib import Path

from easynet_sdk import (
    AdminAgentListRequest,
    AdminAgentStartRequest,
    AdminGatewayStatusRequest,
    AdminJoinHubRequest,
    AdminLeaveHubRequest,
    AdminSessionListRequest,
    CreateDeviceSessionRequest,
    CreatePairingRequest,
    DeleteDeviceSessionRequest,
    DaemonProfileBridge,
    ErrorCode,
    MissionRunRequest,
    MissionRunFileRequest,
    MissionTrackRequest,
    PairingPreflightRequest,
    RevokeDeviceRequest,
    SDKError,
    ValidatePairingRequest,
    VerifyDeviceCredentialRequest,
    is_code,
)


class DaemonProfileBridgeTests(unittest.TestCase):
    def test_admin_facade_dispatches_agent_start_and_list_through_sdk_bridge(
        self,
    ) -> None:
        dispatcher = MemoryProfileDispatcher()
        bridge = DaemonProfileBridge(
            dispatcher, nonce_factory=lambda: bytes(range(1, 17))
        )

        started = bridge.admin_facade().start_agent(
            "assistant",
            kind="codex",
            model="gpt-5",
            label="Assistant",
            command="codex",
            args=("run",),
        )
        records = bridge.admin_facade().list_agents()
        stopped = bridge.admin_facade().stop_agent("assistant")

        self.assertEqual(started.name, "assistant")
        self.assertEqual(started.runtime, "codex")
        self.assertEqual(started.model, "gpt-5")
        self.assertEqual(started.root_path, "/tmp/assistant")
        self.assertTrue(started.replaced_prior)
        self.assertEqual(records[0].name, "assistant")
        self.assertEqual(records[0].runtime, "codex")
        self.assertEqual(stopped.name, "assistant")
        self.assertTrue(stopped.stopped)
        self.assertEqual(dispatcher.calls[0][0], "agent.start")
        self.assertEqual(dispatcher.calls[0][1]["command_args"], ["run"])
        self.assertEqual(dispatcher.calls[1], ("agent.list", {}))
        self.assertEqual(dispatcher.calls[2], ("agent.stop", {"name": "assistant"}))

    def test_mission_facade_dispatches_run_track_cancel_and_events(self) -> None:
        dispatcher = MemoryProfileDispatcher()
        bridge = DaemonProfileBridge(
            dispatcher, nonce_factory=lambda: bytes(range(1, 17))
        )
        mission = bridge.mission_facade()

        run = mission.run_eal('mission "weather" {}\n', label="weather")
        file_run = mission.run_file("/tmp/demo.eal", label="demo")
        tracked = mission.track("run-1")
        cancelled = mission.cancel("run-1")
        events = mission.events("run-1", cursor_sequence=4, limit=25)

        self.assertEqual(run.run_id, "run-1")
        self.assertEqual(file_run.run_id, "run-file-1")
        self.assertEqual(run.run_dir, "/tmp/run-1")
        self.assertEqual(file_run.run_dir, "/tmp/run-file-1")
        self.assertEqual(run.outputs, {"report": "ok"})
        self.assertEqual(tracked["state"], "completed")
        self.assertTrue(cancelled["cancelled"])
        self.assertEqual(events["next_cursor_sequence"], 5)
        self.assertEqual(events["events"][0]["event_type"], "progress")
        self.assertEqual(
            dispatcher.calls,
            [
                (
                    "mission.run",
                    {"source": 'mission "weather" {}\n', "label": "weather"},
                ),
                (
                    "mission.run",
                    {"path": "/tmp/demo.eal", "label": "demo"},
                ),
                ("mission.track", {"run_id": "run-1"}),
                ("mission.cancel", {"run_id": "run-1"}),
                (
                    "mission.events",
                    {"run_id": "run-1", "cursor_sequence": 4, "limit": 25},
                ),
            ],
        )

    def test_admin_profile_dispatches_gateway_trust_and_sessions(self) -> None:
        dispatcher = MemoryProfileDispatcher()
        bridge = DaemonProfileBridge(
            dispatcher, nonce_factory=lambda: bytes(range(1, 17))
        )
        client = bridge.admin_facade()._client  # noqa: SLF001
        base = bridge.admin_base()
        hub = "easynet:///r/example/hub/main"
        device = "easynet:///r/example/device/dev-a"

        status = client.gateway_status(
            AdminGatewayStatusRequest(require_public_listener=True)
        )
        joined = client.join_hub(AdminJoinHubRequest(base, hub, device))
        preflight = client.pairing_preflight(
            PairingPreflightRequest(base, hub, device, ("invoke", "events"))
        )
        token = client.create_pairing(
            CreatePairingRequest(base, hub, device, 1893456000000, ("invoke",))
        )
        credential = client.validate_pairing(
            ValidatePairingRequest(base, "pair-token-value", device)
        )
        verification = client.verify_device_credential(
            VerifyDeviceCredentialRequest(base, "cred-dev-a", device, hub)
        )
        session = client.create_device_session(
            CreateDeviceSessionRequest(
                base, device, hub, "remote_desktop", 1893456000000
            )
        )
        page = client.list_device_sessions(
            AdminSessionListRequest(base, include_terminated=False)
        )
        left = client.leave_hub(AdminLeaveHubRequest(base, hub, reason="rotation"))
        revoked = client.revoke_device(RevokeDeviceRequest(base, device, "rotation"))
        deleted = client.delete_device_session(
            DeleteDeviceSessionRequest(base, "dev-session-1", "done")
        )

        self.assertTrue(status.ready)
        self.assertTrue(status.public_listener_ready)
        self.assertEqual(joined.operation, "hub.join")
        self.assertTrue(joined.ack)
        self.assertTrue(preflight.pairing_required)
        self.assertEqual(token.token_id, "pair-token-1")
        self.assertEqual(credential.credential_id, "cred-dev-a")
        self.assertTrue(verification.verified)
        self.assertEqual(session.session_kind, "remote_desktop")
        self.assertEqual(page.items[0].session_id, "dev-session-1")
        self.assertEqual(left.operation, "hub.leave")
        self.assertEqual(revoked.operation, "federation.revoke")
        self.assertEqual(deleted.operation, "session.delete")
        self.assertEqual(
            dispatcher.calls[-10:],
            [
                ("hub.join", {"hub_ura": hub, "device_ura": device}),
                (
                    "pairing.preflight",
                    {
                        "hub_ura": hub,
                        "device_ura": device,
                        "requested_scopes": ["invoke", "events"],
                    },
                ),
                (
                    "pairing.create",
                    {
                        "hub_ura": hub,
                        "device_ura": device,
                        "expires_unix_ms": 1893456000000,
                        "scopes": ["invoke"],
                    },
                ),
                (
                    "pairing.validate",
                    {"token": "pair-token-value", "device_ura": device},
                ),
                (
                    "credential.verify",
                    {
                        "credential_id": "cred-dev-a",
                        "device_ura": device,
                        "hub_ura": hub,
                    },
                ),
                (
                    "session.create",
                    {
                        "device_ura": device,
                        "hub_ura": hub,
                        "session_kind": "remote_desktop",
                        "expires_unix_ms": 1893456000000,
                    },
                ),
                ("session.list", {"include_terminated": False}),
                ("hub.leave", {"hub_ura": hub, "reason": "rotation"}),
                (
                    "federation.revoke",
                    {"agent_ura": device, "reason": "rotation"},
                ),
                ("session.delete", {"session_id": "dev-session-1", "reason": "done"}),
            ],
        )

    def test_admin_profile_rejects_raw_gateway_status_projection(self) -> None:
        dispatcher = RawGatewayStatusDispatcher()
        bridge = DaemonProfileBridge(
            dispatcher, nonce_factory=lambda: bytes(range(1, 17))
        )

        with self.assertRaises(SDKError) as caught:
            bridge.admin_facade()._client.gateway_status(  # noqa: SLF001
                AdminGatewayStatusRequest()
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn("canonical admin_gateway", caught.exception.message)
        self.assertEqual(dispatcher.calls, [("gateway.status", {})])

    def test_admin_profile_rejects_legacy_device_session_aliases(self) -> None:
        dispatcher = LegacyAdminAliasDispatcher()
        bridge = DaemonProfileBridge(
            dispatcher, nonce_factory=lambda: bytes(range(1, 17))
        )
        base = bridge.admin_base()

        with self.assertRaises(SDKError) as caught:
            bridge.admin_facade()._client.create_device_session(  # noqa: SLF001
                CreateDeviceSessionRequest(
                    base,
                    "easynet:///r/example/device/dev-a",
                    "easynet:///r/example/hub/main",
                    "remote_desktop",
                    1893456000000,
                )
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn("session_id", caught.exception.message)
        self.assertEqual(dispatcher.calls[0][0], "session.create")

    def test_carrier_base_uses_dispatcher_device_and_sdk_nonce(self) -> None:
        bridge = DaemonProfileBridge(
            MemoryProfileDispatcher(), nonce_factory=lambda: bytes(range(1, 17))
        )

        base = bridge.admin_base()

        self.assertEqual(base.caller_ura, "easynet:///r/example/device/dev-a")
        self.assertEqual(base.callee_ura, base.caller_ura)
        self.assertEqual(base.subject_ura, base.caller_ura)
        self.assertEqual(base.nonce_base64, "AQIDBAUGBwgJCgsMDQ4PEA==")

    def test_profile_bridge_builds_admin_and_mission_carriers(self) -> None:
        addressing = RecordingProfileAddressing()
        bridge = DaemonProfileBridge(
            MemoryProfileDispatcher(),
            addressing=addressing,
            nonce_factory=lambda: bytes(range(1, 17)),
        )
        admin = bridge.admin_facade()._client  # noqa: SLF001
        mission = bridge.mission_facade()._client  # noqa: SLF001
        mission_path = Path("/tmp/easynet-sdk-profile-bridge-demo.eal")
        mission_path.write_text("mission demo\nlet r = local.observe_health()\n")
        self.addCleanup(lambda: mission_path.unlink(missing_ok=True))

        agent_list = admin.build_agent_list_invocation(
            AdminAgentListRequest(bridge.admin_base())
        )
        agent_start = admin.build_agent_start_invocation(
            AdminAgentStartRequest(
                bridge.admin_base(),
                name="codex",
                agent_type="codex",
                model="gpt-5",
            )
        )
        run = mission.build_run_eal_invocation(
            MissionRunRequest(
                bridge.mission_base(),
                source="mission demo\nlet r = local.observe_health()",
                label="demo",
            )
        )
        run_file = mission.build_run_file_invocation(
            MissionRunFileRequest(
                bridge.mission_base(), path=str(mission_path), label="file-demo"
            )
        )

        self.assertEqual(agent_list.metadata["carrier_owner"], "daemon_sdk")
        self.assertEqual(agent_list.metadata["system_ability"], "agent.list")
        self.assertEqual(
            agent_list.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0",
        )
        self.assertEqual(agent_start.args["name"], "codex")
        self.assertEqual(agent_start.args["agent_type"], "codex")
        self.assertEqual(run.metadata["system_ability"], "mission.run")
        self.assertEqual(run.args["label"], "demo")
        self.assertEqual(run_file.args["label"], "file-demo")
        self.assertIn("local.observe_health()", run_file.args["source"])
        self.assertEqual(
            addressing.calls,
            [
                ("easynet:///r/example/device/dev-a", "agent.list", "1.0.0"),
                ("easynet:///r/example/device/dev-a", "agent.start", "1.0.0"),
                ("easynet:///r/example/device/dev-a", "mission.run", "1.0.0"),
                ("easynet:///r/example/device/dev-a", "mission.run", "1.0.0"),
            ],
        )

        with self.assertRaises(SDKError) as bad_mission:
            mission.build_track_invocation(
                MissionTrackRequest(bridge.mission_base(), mission_id="/tmp/run")
            )
        self.assertTrue(is_code(bad_mission.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(bad_mission.exception.stage, "mission")

    def test_mission_bad_response_uses_mission_error_stage(self) -> None:
        bridge = DaemonProfileBridge(BadMissionResponseDispatcher())

        with self.assertRaises(SDKError) as caught:
            bridge.mission_facade().track("run-1")

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(caught.exception.stage, "mission")


class MemoryProfileDispatcher:
    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, object]]] = []

    def device_ura(self) -> str:
        return "easynet:///r/example/device/dev-a"

    def invoke_system_ability(
        self, ability: str, **kwargs: object
    ) -> Mapping[str, object]:
        self.calls.append((ability, dict(kwargs)))
        if ability == "agent.start":
            return {
                "state": "ok",
                "model": kwargs.get("model"),
                "root_path": "/tmp/assistant",
                "replaced_prior": True,
            }
        if ability == "agent.list":
            return {
                "agents": [
                    {
                        "name": "assistant",
                        "runtime": "codex",
                        "model": "gpt-5",
                        "metadata": {
                            "root_path": "/tmp/assistant",
                            "root_exists": True,
                            "timeout_secs": 30,
                        },
                    }
                ]
            }
        if ability == "agent.stop":
            return {
                "state": "ok",
                "stopped": True,
                "agent_ura": "easynet:///r/example/agent/assistant",
            }
        if ability == "gateway.status":
            return {
                "profile": "admin_gateway",
                "gateway_id": "gateway-dev-a",
                "ready": True,
                "state": "ready",
                "process_live": True,
                "control_ready": True,
                "runtime_ready": True,
                "directory_ready": True,
                "trust_ready": True,
                "public_listener_ready": True,
                "listeners": [
                    {
                        "kind": "tcp",
                        "endpoint": "127.0.0.1:17337",
                        "ready": True,
                        "public": True,
                    }
                ],
                "identity": {"device_ura": "easynet:///r/example/device/dev-a"},
                "metadata": {
                    "profile": "admin_gateway",
                    "source": "daemon_lifecycle_status",
                    "requires_public_listener": kwargs.get(
                        "require_public_listener", False
                    ),
                },
            }
        if ability == "hub.join":
            return {"state": "ok", "ack": True, "device_ura": kwargs["device_ura"]}
        if ability == "hub.leave":
            return {"state": "ok", "ack": True}
        if ability == "pairing.preflight":
            return {
                "state": "requires_pairing",
                "pairing_required": True,
                "trust_ready": False,
                "scopes": kwargs["requested_scopes"],
            }
        if ability == "pairing.create":
            return {
                "token_id": "pair-token-1",
                "token": "pair-token-value",
                "state": "issued",
                "expires_unix_ms": kwargs["expires_unix_ms"],
                "scopes": kwargs["scopes"],
            }
        if ability == "pairing.validate":
            return {
                "credential_id": "cred-dev-a",
                "device_ura": kwargs["device_ura"],
                "hub_ura": "easynet:///r/example/hub/main",
                "state": "active",
                "issued_unix_ms": 1767225600000,
                "expires_unix_ms": 1893456000000,
                "scopes": ["invoke"],
            }
        if ability == "credential.verify":
            return {
                "verified": True,
                "credential_id": kwargs["credential_id"],
                "device_ura": kwargs["device_ura"],
                "hub_ura": kwargs["hub_ura"],
                "method": "daemon-trust-store",
            }
        if ability == "session.create":
            return {
                "session_id": "dev-session-1",
                "device_ura": kwargs["device_ura"],
                "hub_ura": kwargs["hub_ura"],
                "state": "active",
                "session_kind": kwargs["session_kind"],
                "created_unix_ms": 1767225600000,
                "expires_unix_ms": kwargs["expires_unix_ms"],
            }
        if ability == "session.list":
            return {
                "sessions": [
                    {
                        "session_id": "dev-session-1",
                        "device_ura": "easynet:///r/example/device/dev-a",
                        "hub_ura": "easynet:///r/example/hub/main",
                        "state": "active",
                        "session_kind": "remote_desktop",
                        "created_unix_ms": 1767225600000,
                        "expires_unix_ms": 1893456000000,
                    }
                ]
            }
        if ability == "federation.revoke":
            return {"state": "ok", "ack": True, "device_ura": kwargs["agent_ura"]}
        if ability == "session.delete":
            return {"state": "deleted", "ack": True}
        if ability == "mission.run":
            if "path" in kwargs:
                return {
                    "run_id": "run-file-1",
                    "run_dir": "/tmp/run-file-1",
                    "outputs": {"file": kwargs["path"]},
                    "state": "running",
                }
            return {
                "run_id": "run-1",
                "run_dir": "/tmp/run-1",
                "outputs": {"report": "ok"},
                "state": "running",
            }
        if ability == "mission.track":
            return {
                "run_id": "run-1",
                "run_dir": "/tmp/run-1",
                "outputs": {"report": "ok"},
                "state": "completed",
                "terminal": True,
            }
        if ability == "mission.cancel":
            return {
                "run_id": "run-1",
                "state": "cancelled",
                "cancelled": True,
                "terminal": True,
            }
        if ability == "mission.events":
            return {
                "cursor_sequence": kwargs["cursor_sequence"],
                "next_cursor_sequence": 5,
                "has_more": False,
                "dropped_count": 0,
                "events": [
                    {
                        "sequence": 4,
                        "event_type": "progress",
                        "occurred_unix_ms": 1783126923000,
                        "terminal": False,
                        "payload": {"step": "s1"},
                        "receipt": {},
                        "metadata": {"source": "test"},
                    }
                ],
            }
        raise AssertionError(f"unexpected ability {ability}")


class RawGatewayStatusDispatcher(MemoryProfileDispatcher):
    def invoke_system_ability(
        self, ability: str, **kwargs: object
    ) -> Mapping[str, object]:
        self.calls.append((ability, dict(kwargs)))
        if ability == "gateway.status":
            return {
                "gateway_id": "legacy-gateway",
                "ready": True,
                "running": True,
                "state": "ready",
            }
        raise AssertionError(f"unexpected ability {ability}")


class LegacyAdminAliasDispatcher(MemoryProfileDispatcher):
    def invoke_system_ability(
        self, ability: str, **kwargs: object
    ) -> Mapping[str, object]:
        self.calls.append((ability, dict(kwargs)))
        if ability == "session.create":
            return {
                "sessionId": "dev-session-1",
                "deviceUra": kwargs["device_ura"],
                "hubUra": kwargs["hub_ura"],
                "status": "active",
                "sessionKind": kwargs["session_kind"],
                "createdUnixMs": 1767225600000,
                "expiresUnixMs": kwargs["expires_unix_ms"],
            }
        raise AssertionError(f"unexpected ability {ability}")


class RecordingProfileAddressing:
    def __init__(self) -> None:
        self.calls: list[tuple[str, str, str]] = []

    def owner_ability_descriptor_ref(
        self,
        owner_ura: str,
        ability_name: str,
        descriptor_version: str = "1.0.0",
    ) -> str:
        self.calls.append((owner_ura, ability_name, descriptor_version))
        owner_tail = owner_ura.removeprefix("easynet:///r/example/device/")
        return (
            f"easynet:///r/example/ability/device.{owner_tail}."
            f"{ability_name}@{descriptor_version}"
        )


class BadMissionResponseDispatcher(MemoryProfileDispatcher):
    def invoke_system_ability(
        self, ability: str, **kwargs: object
    ) -> Mapping[str, object]:
        if ability == "mission.track":
            return {
                "run_id": "run-1",
                "state": "completed",
                "terminal": True,
                "parent_invocation_id": 42,
            }
        return super().invoke_system_ability(ability, **kwargs)


if __name__ == "__main__":
    unittest.main()
