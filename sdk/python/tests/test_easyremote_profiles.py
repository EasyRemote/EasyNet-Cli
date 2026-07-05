import unittest
from collections.abc import Mapping

from easynet_sdk import (
    AdminAgentListRequest,
    EasyRemoteProfileBridge,
    ErrorCode,
    MissionRunFileRequest,
    SDKError,
    is_code,
)


class EasyRemoteProfileBridgeTests(unittest.TestCase):
    def test_admin_facade_dispatches_agent_start_and_list_through_sdk_bridge(
        self,
    ) -> None:
        dispatcher = MemoryProfileDispatcher()
        bridge = EasyRemoteProfileBridge(
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
        bridge = EasyRemoteProfileBridge(
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

    def test_carrier_base_uses_dispatcher_device_and_sdk_nonce(self) -> None:
        bridge = EasyRemoteProfileBridge(
            MemoryProfileDispatcher(), nonce_factory=lambda: bytes(range(1, 17))
        )

        base = bridge.admin_base()

        self.assertEqual(base.caller_ura, "easynet:///r/example/device/dev-a")
        self.assertEqual(base.callee_ura, base.caller_ura)
        self.assertEqual(base.subject_ura, base.caller_ura)
        self.assertEqual(base.nonce_base64, "AQIDBAUGBwgJCgsMDQ4PEA==")

    def test_unsupported_profile_operations_fail_closed(self) -> None:
        bridge = EasyRemoteProfileBridge(MemoryProfileDispatcher())

        with self.assertRaises(SDKError) as caught:
            bridge.admin_facade()._client.build_agent_list_invocation(  # noqa: SLF001
                AdminAgentListRequest(bridge.admin_base())
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.NOT_IMPLEMENTED))
        self.assertEqual(
            caught.exception.details["profile"],
            "easyremote_admin_profile",
        )
        self.assertEqual(
            caught.exception.details["source_ref"],
            "python_sdk.profile.easyremote_admin_profile",
        )
        self.assertEqual(
            caught.exception.details["profile_method"],
            "build_agent_list_invocation",
        )

        with self.assertRaises(SDKError) as mission_caught:
            bridge.mission_facade()._client.build_run_file_invocation(  # noqa: SLF001
                MissionRunFileRequest(bridge.mission_base(), path="/tmp/demo.eal")
            )

        self.assertTrue(is_code(mission_caught.exception, ErrorCode.NOT_IMPLEMENTED))
        self.assertEqual(
            mission_caught.exception.details["profile"],
            "easyremote_mission_profile",
        )
        self.assertEqual(
            mission_caught.exception.details["source_ref"],
            "python_sdk.profile.easyremote_mission_profile",
        )
        self.assertEqual(
            mission_caught.exception.details["profile_method"],
            "build_run_file_invocation",
        )

    def test_mission_bad_response_uses_mission_error_stage(self) -> None:
        bridge = EasyRemoteProfileBridge(BadMissionResponseDispatcher())

        with self.assertRaises(SDKError) as caught:
            bridge.mission_facade().track("run-1")

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(caught.exception.stage, "easyremote_mission_profile")


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
