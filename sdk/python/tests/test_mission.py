import json
import unittest

from easynet_sdk import ErrorCode, SDKError, is_code
from easynet_sdk.mission import (
    MissionCancelRequest,
    MissionCarrierBase,
    MissionClient,
    MissionEventListRequest,
    MissionRunFileRequest,
    MissionRunRequest,
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


class MemoryMissionTransport:
    def __init__(self) -> None:
        self.run_invocation_json = MISSION_RUN_INVOCATION_JSON
        self.run_file_invocation_json = MISSION_RUN_INVOCATION_JSON
        self.track_invocation_json = MISSION_TRACK_INVOCATION_JSON
        self.cancel_invocation_json = MISSION_CANCEL_INVOCATION_JSON
        self.status_json = MISSION_STATUS_JSON
        self.events_json = MISSION_EVENT_PAGE_JSON
        self.seen_request: dict[str, object] | None = None
        self.close_calls = 0

    def _remember(self, request_json: bytes) -> None:
        self.seen_request = json.loads(request_json.decode("utf-8"))

    def build_run_eal_invocation(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.run_invocation_json

    def build_run_file_invocation(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.run_file_invocation_json

    def build_track_invocation(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.track_invocation_json

    def build_cancel_invocation(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.cancel_invocation_json

    def run_eal(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.status_json

    def run_file(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.status_json

    def track(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.status_json

    def cancel(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.status_json

    def events(self, request_json: bytes) -> bytes:
        self._remember(request_json)
        return self.events_json

    def close(self) -> None:
        self.close_calls += 1


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
