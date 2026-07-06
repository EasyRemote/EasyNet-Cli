import json
import unittest
from dataclasses import replace

from easynet_sdk import MissionExecutionAdapter, ErrorCode, SDKError, StreamHandle, is_code
from easynet_sdk.mission import (
    MissionPlan,
    MissionCancelRequest,
    MissionCarrierBase,
    MissionClient,
    MissionEventListRequest,
    MissionEventTailOptions,
    MissionRunFileRequest,
    MissionRunRequest,
    MissionStatus,
    MissionTrackRequest,
)


MISSION_RUN_INVOCATION_JSON = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.mission.run@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"source": "mission weather\\nlet r = local.observe_health()", "label": "weather"},
  "content_type": "application/json",
  "metadata": {
    "request_id": "mission-run-1",
    "profile": "mission",
    "system_ability": "mission.run",
    "carrier_owner": "daemon_sdk"
  }
}"""

MISSION_TRACK_INVOCATION_JSON = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.mission.track@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"run_id": "2026-07-04_010203_weather"},
  "content_type": "application/json",
  "metadata": {"request_id": "mission-track-1", "profile": "mission", "system_ability": "mission.track", "carrier_owner": "daemon_sdk"}
}"""

MISSION_CANCEL_INVOCATION_JSON = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.mission.cancel@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"run_id": "2026-07-04_010203_weather"},
  "content_type": "application/json",
  "metadata": {"request_id": "mission-cancel-1", "profile": "mission", "system_ability": "mission.cancel", "carrier_owner": "daemon_sdk"}
}"""

MISSION_STATUS_JSON = b"""{
  "profile": "mission",
  "kind": "mission_status",
  "mission_id": "2026-07-04_010203_weather",
  "state": "partial",
  "terminal": true,
  "partial_failures": 1,
  "cancelled": false,
  "parent_invocation_id": null,
  "parent_receipt_ura": "easynet:///r/example/receipt/parent",
  "parent_invocation": {"caller": "easynet:///r/example/agent/alice.sdk"},
  "child_invocations": [
    {
      "step_id": "s1",
      "request_id": "req-1",
      "trace_id": "2026-07-04_010203_weather",
      "ability": "observe.health",
      "invocation_ura": "easynet:///r/example/invocation/req-1",
      "caller_ura": "easynet:///r/example/device/dev-a",
      "callee_ura": "easynet:///r/example/device/dev-a",
      "subject_ura": "easynet:///r/example/device/dev-a",
      "metadata_state": "receipt_backed",
      "ledger_state": "completed",
      "receipt": {"receipt_ura": "easynet:///r/example/receipt/child", "receipt_hash": "bbbb", "head_receipt_hash": "bbbb"}
    }
  ],
  "child_receipts": [{"step_id": "s1", "invocation_ura": "easynet:///r/example/invocation/req-1", "receipt_ura": "easynet:///r/example/receipt/child", "receipt_hash": "bbbb"}],
  "output_refs": [{"kind": "run_dir", "path": "/tmp/easynet/missions/runs/2026-07-04_010203_weather"}],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}"""

MISSION_ADAPTER_RUN_STATUS_JSON = b"""{
  "profile": "mission",
  "kind": "mission_status",
  "mission_id": "run-1",
  "state": "running",
  "terminal": false,
  "partial_failures": 0,
  "cancelled": false,
  "parent_invocation_id": "invoke-run-1",
  "parent_receipt_ura": null,
  "parent_invocation": null,
  "child_invocations": [],
  "child_receipts": [],
  "output_refs": [{"kind": "run_dir", "path": "/tmp/run-1", "metadata": {}}],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk", "outputs": {"a": 1}}
}"""

MISSION_ADAPTER_TRACK_STATUS_JSON = b"""{
  "profile": "mission",
  "kind": "mission_status",
  "mission_id": "run-1",
  "state": "running",
  "terminal": false,
  "partial_failures": 0,
  "cancelled": false,
  "parent_invocation_id": "invoke-run-1",
  "parent_receipt_ura": null,
  "parent_invocation": null,
  "child_invocations": [],
  "child_receipts": [],
  "output_refs": [{"kind": "run_dir", "path": "/tmp/run-1", "metadata": {}}],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}"""

MISSION_ADAPTER_CANCEL_STATUS_JSON = b"""{
  "profile": "mission",
  "kind": "mission_status",
  "mission_id": "run-1",
  "state": "cancelled",
  "terminal": true,
  "partial_failures": 0,
  "cancelled": true,
  "parent_invocation_id": "invoke-run-1",
  "parent_receipt_ura": null,
  "parent_invocation": null,
  "child_invocations": [],
  "child_receipts": [],
  "output_refs": [{"kind": "run_dir", "path": "/tmp/run-1", "metadata": {}}],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}"""

MISSION_EVENT_PAGE_JSON = b"""{
  "profile": "mission",
  "kind": "mission_event_page",
  "mission_id": "2026-07-04_010203_weather",
  "cursor_sequence": 4,
  "next_cursor_sequence": 7,
  "has_more": false,
  "dropped_count": 0,
  "events": [
    {
      "profile": "mission",
      "kind": "mission_event",
      "mission_id": "2026-07-04_010203_weather",
      "sequence": 4,
      "event_type": "progress",
      "occurred_unix_ms": 1004,
      "terminal": false,
      "payload": {"delta": "hello"},
      "receipt": {},
      "metadata": {"step_id": "s1"}
    },
    {
      "profile": "mission",
      "kind": "mission_event",
      "mission_id": "2026-07-04_010203_weather",
      "sequence": 6,
      "event_type": "completed",
      "occurred_unix_ms": 1006,
      "terminal": true,
      "payload": {"reply": "done"},
      "receipt": {"receipt_ura": "easynet:///r/example/receipt/terminal"},
      "metadata": {}
    }
  ],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}"""

MISSION_EVENT_TAIL_PAGE_1_JSON = b"""{
  "profile": "mission",
  "kind": "mission_event_page",
  "mission_id": "run-1",
  "cursor_sequence": 0,
  "next_cursor_sequence": 1,
  "has_more": true,
  "dropped_count": 0,
  "events": [
    {
      "profile": "mission",
      "kind": "mission_event",
      "mission_id": "run-1",
      "sequence": 0,
      "event_type": "progress",
      "occurred_unix_ms": 1000,
      "terminal": false,
      "payload": {"step": "fetch"},
      "receipt": {},
      "metadata": {"step_id": "fetch"}
    }
  ],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}"""

MISSION_EVENT_TAIL_PAGE_2_JSON = b"""{
  "profile": "mission",
  "kind": "mission_event_page",
  "mission_id": "run-1",
  "cursor_sequence": 1,
  "next_cursor_sequence": 2,
  "has_more": false,
  "dropped_count": 0,
  "events": [
    {
      "profile": "mission",
      "kind": "mission_event",
      "mission_id": "run-1",
      "sequence": 1,
      "event_type": "completed",
      "occurred_unix_ms": 1001,
      "terminal": true,
      "payload": {"reply": "done"},
      "receipt": {"receipt_ura": "easynet:///r/example/receipt/terminal"},
      "metadata": {}
    }
  ],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}"""

MISSION_EVENT_TAIL_TERMINAL_THEN_STRAY_PAGE_JSON = b"""{
  "profile": "mission",
  "kind": "mission_event_page",
  "mission_id": "run-1",
  "cursor_sequence": 0,
  "next_cursor_sequence": 2,
  "has_more": false,
  "dropped_count": 0,
  "events": [
    {
      "profile": "mission",
      "kind": "mission_event",
      "mission_id": "run-1",
      "sequence": 0,
      "event_type": "completed",
      "occurred_unix_ms": 1000,
      "terminal": true,
      "payload": {"reply": "done"},
      "receipt": {"receipt_ura": "easynet:///r/example/receipt/terminal"},
      "metadata": {}
    },
    {
      "profile": "mission",
      "kind": "mission_event",
      "mission_id": "run-1",
      "sequence": 1,
      "event_type": "progress",
      "occurred_unix_ms": 1001,
      "terminal": false,
      "payload": {"delta": "stray"},
      "receipt": {},
      "metadata": {}
    }
  ],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}"""

MISSION_EVENT_DROPPED_PAGE_JSON = b"""{
  "profile": "mission",
  "kind": "mission_event_page",
  "mission_id": "run-1",
  "cursor_sequence": 0,
  "next_cursor_sequence": 3,
  "has_more": false,
  "dropped_count": 2,
  "events": [],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}"""


class MemoryMissionTransport:
    def __init__(self) -> None:
        self.run_invocation_json = MISSION_RUN_INVOCATION_JSON
        self.run_file_invocation_json = MISSION_RUN_INVOCATION_JSON
        self.track_invocation_json = MISSION_TRACK_INVOCATION_JSON
        self.cancel_invocation_json = MISSION_CANCEL_INVOCATION_JSON
        self.run_status_json = MISSION_STATUS_JSON
        self.run_file_status_json = MISSION_STATUS_JSON
        self.track_status_json = MISSION_STATUS_JSON
        self.cancel_status_json = MISSION_STATUS_JSON
        self.events_json = MISSION_EVENT_PAGE_JSON
        self.events_jsons: list[bytes] = []
        self.stream_events: list[bytes] = []
        self.stream_cancel_reason = ""
        self.stream_close_calls = 0
        self.seen: dict[str, dict[str, object]] = {}
        self.seen_calls: dict[str, list[dict[str, object]]] = {}
        self.seen_request: dict[str, object] | None = None
        self.close_calls = 0

    def _remember(self, name: str, request_json: bytes) -> None:
        decoded = json.loads(request_json.decode("utf-8"))
        self.seen[name] = decoded
        self.seen_calls.setdefault(name, []).append(decoded)
        self.seen_request = decoded

    def build_run_eal_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_run_eal_invocation", request_json)
        return self.run_invocation_json

    def build_run_file_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_run_file_invocation", request_json)
        return self.run_file_invocation_json

    def build_track_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_track_invocation", request_json)
        return self.track_invocation_json

    def build_cancel_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_cancel_invocation", request_json)
        return self.cancel_invocation_json

    def run_eal(self, request_json: bytes) -> bytes:
        self._remember("run_eal", request_json)
        return self.run_status_json

    def run_file(self, request_json: bytes) -> bytes:
        self._remember("run_file", request_json)
        return self.run_file_status_json

    def track(self, request_json: bytes) -> bytes:
        self._remember("track", request_json)
        return self.track_status_json

    def cancel(self, request_json: bytes) -> bytes:
        self._remember("cancel", request_json)
        return self.cancel_status_json

    def events(self, request_json: bytes) -> bytes:
        self._remember("events", request_json)
        if self.events_jsons:
            return self.events_jsons.pop(0)
        return self.events_json

    def open_event_stream(self, request_json: bytes) -> StreamHandle:
        self._remember("open_event_stream", request_json)
        return StreamHandle.from_json(
            _MemoryMissionStreamTransport(self),
            b'{"stream_id":"mission-events-1","state":"Open","max_buffered_events":8}',
        )

    def close(self) -> None:
        self.close_calls += 1


class _MemoryMissionStreamTransport:
    def __init__(self, mission_transport: MemoryMissionTransport) -> None:
        self._mission_transport = mission_transport

    def recv(self, timeout: float | None = None) -> bytes:
        if self._mission_transport.stream_events:
            return self._mission_transport.stream_events.pop(0)
        return b"""{
          "sequence": 1,
          "kind": "data",
          "state": "Open",
          "payload_json": {
            "profile": "mission",
            "kind": "mission_event",
            "mission_id": "2026-07-04_010203_weather",
            "sequence": 7,
            "event_type": "progress",
            "occurred_unix_ms": 1007,
            "terminal": false,
            "payload": {"delta": "stream"},
            "receipt": {},
            "metadata": {"step_id": "s1"}
          }
        }"""

    def cancel(self, reason: str) -> bytes:
        self._mission_transport.stream_cancel_reason = reason
        return b'{"stream_id":"mission-events-1","cancelled":true,"state":"Cancelled","terminal":true}'

    def close(self) -> None:
        self._mission_transport.stream_close_calls += 1


def base() -> MissionCarrierBase:
    return MissionCarrierBase(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/device/dev-a",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        metadata={"request_id": "mission-run-1"},
    )


class MissionTests(unittest.TestCase):
    def test_mission_plan_renders_eal_and_child_intents(self) -> None:
        plan = MissionPlan(
            "nightly-report",
            created_by="easynet:///r/example/device/dev-a",
            version="0.9.0",
        )
        fetch = plan.step("teamA.fetch_sales", args={"quarter": "Q2"}, timeout=60)
        plan.step("er.summarize", args={"rows": fetch.output}, retries=2, on_failure="retry")

        eal = plan.to_eal()
        intents = plan.child_invocation_intents()

        self.assertIn("// generated by easynet daemon sdk 0.9.0", eal)
        self.assertIn("// created_by: easynet:///r/example/device/dev-a", eal)
        self.assertIn('let fetch_sales = call "teamA.fetch_sales"', eal)
        self.assertIn("rows = fetch_sales.output", eal)
        self.assertEqual([intent.step_id for intent in intents], ["fetch_sales", "summarize"])
        self.assertEqual(intents[0].ability, "teamA.fetch_sales")

    def test_mission_plan_rejects_invalid_fields(self) -> None:
        plan = MissionPlan("p")
        with self.assertRaises(SDKError):
            plan.step("er.fn", timeout=0)
        with self.assertRaises(SDKError):
            plan.step("er.fn", args={"payload": {"nested": 1}})

        foreign = MissionPlan("other").step("er.src")
        with self.assertRaises(SDKError) as caught:
            plan.step("er.fn", args={"data": foreign.output})
        self.assertIn("not part of this mission plan", caught.exception.message)

    def test_mission_plan_validates_child_invocation_facts(self) -> None:
        plan = MissionPlan("nightly")
        plan.step("observe.health")
        status = MissionStatus.from_json(
            MISSION_STATUS_JSON.replace(b'"step_id": "s1"', b'"step_id": "health"')
        )

        result = plan.validate_child_invocations(status)

        self.assertTrue(result.passed)
        self.assertEqual(result.expected_steps, ("health",))
        self.assertEqual(result.receipt_backed_steps, ("health",))

        missing = MissionPlan("nightly")
        missing.step("observe.health")
        missing.step("notify.user")
        with self.assertRaises(SDKError) as caught:
            missing.validate_child_invocations(status)
        self.assertTrue(is_code(caught.exception, ErrorCode.PROTOCOL))
        self.assertEqual(
            caught.exception.details["reason"],
            "mission_child_invocation_mismatch",
        )

        with self.assertRaises(SDKError) as mismatch:
            plan.validate_child_invocations(
                MissionStatus.from_json(
                    MISSION_STATUS_JSON.replace(
                        b'"step_id": "s1"', b'"step_id": "health"'
                    ).replace(
                        b'"ability": "observe.health"', b'"ability": "observe.other"'
                    )
                )
            )
        self.assertEqual(mismatch.exception.details["ability_mismatched_steps"], ["health"])

        incomplete_child = replace(status.child_invocations[0], invocation_ura=None)
        incomplete_status = replace(status, child_invocations=(incomplete_child,))
        with self.assertRaises(SDKError) as incomplete:
            plan.validate_child_invocations(incomplete_status)
        self.assertEqual(
            incomplete.exception.details["incomplete_fact_steps"], ["health"]
        )

    def test_mission_status_rejects_incomplete_child_invocation_fact(self) -> None:
        with self.assertRaises(SDKError) as caught:
            MissionStatus.from_json(
                MISSION_STATUS_JSON.replace(b'"request_id": "req-1"', b'"request_id": null')
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

        with self.assertRaises(SDKError) as receipt:
            MissionStatus.from_json(
                MISSION_STATUS_JSON.replace(b'"receipt_hash": "bbbb"', b'"receipt_hash": ""')
            )
        self.assertTrue(is_code(receipt.exception, ErrorCode.INVALID_ARGUMENT))

    def test_mission_execution_adapter_runs_tracks_and_cancels_missions(self) -> None:
        transport = MemoryMissionTransport()
        transport.run_status_json = MISSION_ADAPTER_RUN_STATUS_JSON
        transport.track_status_json = MISSION_ADAPTER_TRACK_STATUS_JSON
        transport.cancel_status_json = MISSION_ADAPTER_CANCEL_STATUS_JSON
        client = MissionClient(transport)
        adapter = MissionExecutionAdapter(client, base())

        run = adapter.run_eal('mission "nightly" {}\n', label="nightly")
        tracked = adapter.track("run-1")
        cancelled = adapter.cancel("run-1")

        self.assertEqual(run.run_id, "run-1")
        self.assertEqual(run.run_dir, "/tmp/run-1")
        self.assertEqual(run.outputs, {"a": 1})
        self.assertEqual(tracked["state"], "running")
        self.assertEqual(cancelled["cancelled"], True)
        self.assertEqual(
            transport.seen["run_eal"]["caller_ura"],
            "easynet:///r/example/agent/alice.sdk",
        )
        self.assertEqual(transport.seen["run_eal"]["source"], 'mission "nightly" {}\n')
        self.assertEqual(transport.seen["run_eal"]["label"], "nightly")
        self.assertEqual(transport.seen["track"]["mission_id"], "run-1")
        self.assertEqual(transport.seen["cancel"]["mission_id"], "run-1")

    def test_mission_execution_adapter_exposes_mission_events(self) -> None:
        transport = MemoryMissionTransport()
        adapter = MissionExecutionAdapter(MissionClient(transport), base())

        page = adapter.events("run-1", cursor_sequence=4, limit=100)

        self.assertEqual(page["next_cursor_sequence"], 7)
        self.assertEqual(page["events"][0]["event_type"], "progress")
        self.assertTrue(page["events"][1]["terminal"])
        self.assertEqual(transport.seen["events"]["mission_id"], "run-1")
        self.assertEqual(transport.seen["events"]["cursor_sequence"], 4)
        self.assertEqual(transport.seen["events"]["limit"], 100)

    def test_mission_execution_adapter_tails_mission_events_until_terminal(self) -> None:
        transport = MemoryMissionTransport()
        transport.events_jsons = [
            MISSION_EVENT_TAIL_PAGE_1_JSON,
            MISSION_EVENT_TAIL_PAGE_2_JSON,
        ]
        adapter = MissionExecutionAdapter(MissionClient(transport), base())

        tail = adapter.tail_events("run-1", cursor_sequence=0, limit=10)
        events = list(tail)

        self.assertEqual([event["event_type"] for event in events], ["progress", "completed"])
        self.assertEqual(events[1]["payload"], {"reply": "done"})
        self.assertEqual(tail.cursor_sequence, 2)
        self.assertEqual(
            [call["cursor_sequence"] for call in transport.seen_calls["events"]],
            [0, 1],
        )
        self.assertEqual(transport.seen_calls["events"][0]["limit"], 10)

    def test_mission_execution_adapter_tail_reports_dropped_events(self) -> None:
        transport = MemoryMissionTransport()
        transport.events_jsons = [MISSION_EVENT_DROPPED_PAGE_JSON]
        adapter = MissionExecutionAdapter(MissionClient(transport), base())

        with self.assertRaises(SDKError) as raised:
            list(adapter.tail_events("run-1"))

        self.assertTrue(is_code(raised.exception, ErrorCode.PROTOCOL))
        self.assertEqual(raised.exception.details["reason"], "mission_events_dropped")

    def test_builds_run_track_cancel_invocations(self) -> None:
        client = MissionClient(MemoryMissionTransport())

        run = client.build_run_eal_invocation(
            MissionRunRequest(
                base=base(),
                source="mission weather\nlet r = local.observe_health()",
                label="weather",
            )
        )
        self.assertEqual(
            run.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.mission.run@1.0.0",
        )

        track = client.build_track_invocation(
            MissionTrackRequest(base=base(), mission_id="2026-07-04_010203_weather")
        )
        self.assertEqual(
            track.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.mission.track@1.0.0",
        )

        cancel = client.build_cancel_invocation(
            MissionCancelRequest(base=base(), mission_id="2026-07-04_010203_weather")
        )
        self.assertEqual(
            cancel.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.mission.cancel@1.0.0",
        )

    def test_run_file_and_status_projection(self) -> None:
        transport = MemoryMissionTransport()
        client = MissionClient(transport)

        client.build_run_file_invocation(
            MissionRunFileRequest(
                base=base(),
                path="/tmp/easynet-sdk-demo.eal",
                label="file-weather",
            )
        )
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["path"], "/tmp/easynet-sdk-demo.eal")

        status = client.track(
            MissionTrackRequest(base=base(), mission_id="2026-07-04_010203_weather")
        )
        self.assertTrue(status.terminal)
        self.assertEqual(status.state, "partial")
        self.assertEqual(status.child_receipts[0].receipt_ura, "easynet:///r/example/receipt/child")
        self.assertEqual(status.output_refs[0].kind, "run_dir")

    def test_events_projection(self) -> None:
        transport = MemoryMissionTransport()
        client = MissionClient(transport)

        page = client.events(
            MissionEventListRequest(
                base=base(),
                mission_id="2026-07-04_010203_weather",
                cursor_sequence=4,
                limit=100,
            )
        )

        self.assertEqual(page.kind, "mission_event_page")
        self.assertEqual(page.next_cursor_sequence, 7)
        self.assertEqual(len(page.events), 2)
        self.assertEqual(page.events[0].event_type, "progress")
        self.assertTrue(page.events[1].terminal)
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["mission_id"], "2026-07-04_010203_weather")
        self.assertEqual(transport.seen_request["cursor_sequence"], 4)

    def test_client_opens_runtime_event_stream(self) -> None:
        transport = MemoryMissionTransport()
        client = MissionClient(transport)

        stream = client.open_event_stream(
            MissionEventListRequest(
                base=base(),
                mission_id="2026-07-04_010203_weather",
                cursor_sequence=7,
                limit=10,
            )
        )
        event = stream.next()
        cancel = stream.cancel("done")
        stream.close()

        self.assertEqual(stream.stream_id, "mission-events-1")
        self.assertEqual(event.event_type, "progress")
        self.assertEqual(event.payload, {"delta": "stream"})
        self.assertEqual(transport.seen["open_event_stream"]["mission_id"], "2026-07-04_010203_weather")
        self.assertEqual(transport.seen["open_event_stream"]["cursor_sequence"], 7)
        self.assertEqual(transport.seen["open_event_stream"]["limit"], 10)
        self.assertTrue(cancel.cancelled)
        self.assertEqual(transport.stream_cancel_reason, "done")
        self.assertEqual(transport.stream_close_calls, 1)

    def test_mission_event_stream_rejects_missing_payload(self) -> None:
        transport = MemoryMissionTransport()
        transport.stream_events = [b'{"sequence":1,"kind":"data","state":"Open"}']
        client = MissionClient(transport)

        stream = client.open_event_stream(
            MissionEventListRequest(
                base=base(),
                mission_id="2026-07-04_010203_weather",
            )
        )

        with self.assertRaises(SDKError) as caught:
            stream.next()
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_client_tails_mission_events_until_terminal(self) -> None:
        transport = MemoryMissionTransport()
        transport.events_jsons = [
            MISSION_EVENT_TAIL_PAGE_1_JSON,
            MISSION_EVENT_TAIL_PAGE_2_JSON,
        ]
        client = MissionClient(transport)

        tail = client.tail_events(
            MissionEventListRequest(
                base=base(),
                mission_id="2026-07-04_010203_weather",
            ),
            options=MissionEventTailOptions(limit=10),
        )
        events = list(tail)

        self.assertEqual([event.event_type for event in events], ["progress", "completed"])
        self.assertEqual(events[1].payload, {"reply": "done"})
        self.assertEqual(tail.cursor_sequence, 2)
        self.assertEqual(
            [call["cursor_sequence"] for call in transport.seen_calls["events"]],
            [0, 1],
        )
        self.assertEqual(transport.seen_calls["events"][0]["limit"], 10)

    def test_client_tail_stops_within_page_after_terminal(self) -> None:
        transport = MemoryMissionTransport()
        transport.events_jsons = [MISSION_EVENT_TAIL_TERMINAL_THEN_STRAY_PAGE_JSON]
        client = MissionClient(transport)

        tail = client.tail_events(
            MissionEventListRequest(base=base(), mission_id="2026-07-04_010203_weather")
        )

        events = list(tail)

        self.assertEqual([event.event_type for event in events], ["completed"])
        self.assertEqual(tail.cursor_sequence, 2)

    def test_rejects_incomplete_carrier_and_path_like_mission_id(self) -> None:
        client = MissionClient(MemoryMissionTransport())
        bad_base = MissionCarrierBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
        )
        with self.assertRaises(SDKError):
            client.build_run_eal_invocation(MissionRunRequest(base=bad_base, source="mission x"))

        with self.assertRaises(SDKError):
            client.build_track_invocation(MissionTrackRequest(base=base(), mission_id="/tmp/run"))

        with self.assertRaises(SDKError):
            client.events(
                MissionEventListRequest(
                    base=base(),
                    mission_id="2026-07-04_010203_weather",
                    cursor_sequence=-1,
                )
            )

    def test_close_delegates_once_and_fails_closed(self) -> None:
        transport = MemoryMissionTransport()
        client = MissionClient(transport)

        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            client.build_run_eal_invocation(
                MissionRunRequest(
                    base=base(),
                    source="mission weather\nlet r = local.observe_health()",
                    label="weather",
                )
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(transport.seen_request)


if __name__ == "__main__":
    unittest.main()
