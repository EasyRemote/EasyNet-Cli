import json
import unittest

from easynet_sdk import ErrorCode, SDKError, is_code
from easynet_sdk.events import (
    DEFAULT_EVENT_PAGE_SIZE,
    MAX_EVENT_PAGE_SIZE,
    DeviceEventPage,
    EventClient,
    EventCursor,
    EventDropReportInput,
    EventFilter,
    EventFrame,
    EventProjectionInput,
    EventStream,
    EventTerminalInput,
    EventsCarrierBase,
    EventsDeviceEventListRequest,
    DeviceEventQuery,
    DirectoryEventQuery,
    InvocationEventQuery,
    SessionEventQuery,
)
from easynet_sdk.stream import StreamHandle


EVENTS_DIRECTORY_SUBSCRIPTION_INVOCATION = b"""{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {
    "stream": "directory",
    "daemon_ability": "federation.subscribe_directory_v2",
    "realm": "example",
    "agent_ura": "easynet:///r/example/agent/alice.main",
    "resume_cursor": "directory:7",
    "heartbeat_interval_ms": 30000
  },
  "content_type": "application/json",
  "metadata": {
    "request_id": "events-directory-subscribe-1",
    "profile": "events",
    "system_ability": "federation.subscribe_directory_v2",
    "carrier_owner": "daemon_sdk"
  }
}"""

EVENTS_DIRECTORY_EVENT = b"""{
  "profile": "events",
  "stream": "directory",
  "kind": "directory.agent_advertised",
  "event_id": "evt-directory-8",
  "cursor": {"stream": "directory", "sequence": 8, "token": "directory:8"},
  "resume_token": "directory:8",
  "occurred_unix_ms": 1783100000123,
  "occurred_at": "2026-07-03T17:33:20.123Z",
  "subject_ref": {"kind": "ura", "ura": "easynet:///r/example/agent/alice.main", "role": "agent"},
  "tenant_ref": {"kind": "realm", "realm": "example"},
  "payload": {
    "type": "agent_advertised",
    "agent_ura": "easynet:///r/example/agent/alice.main",
    "signing_authority": "self_signed",
    "replaced_prior": false,
    "unix_ms": 1783100000123
  },
  "dropped_count": 0,
  "reconnect_after_ms": null,
  "terminal": false,
  "metadata": {
    "profile": "events",
    "stream": "directory",
    "carrier_owner": "daemon_sdk",
    "source": "daemon_directory_event",
    "stream_ability": "federation.subscribe_directory_v2",
    "lifecycle": "delta",
    "daemon_event_type": "agent_advertised"
  }
}"""

EVENTS_DROP_REPORT = b"""{
  "profile": "events",
  "stream": "directory",
  "kind": "directory.drop_report",
  "event_id": "evt-directory-10",
  "cursor": {"stream": "directory", "sequence": 10, "token": "directory:10"},
  "resume_token": "resnapshot",
  "occurred_unix_ms": 1783100000123,
  "occurred_at": "2026-07-03T17:33:20.123Z",
  "subject_ref": null,
  "tenant_ref": null,
  "payload": {"reason": "consumer_lagged", "dropped_count": 4},
  "dropped_count": 4,
  "reconnect_after_ms": 1000,
  "terminal": false,
  "metadata": {
    "profile": "events",
    "stream": "directory",
    "carrier_owner": "daemon_sdk",
    "source": "daemon_directory_event",
    "stream_ability": "federation.subscribe_directory_v2",
    "lifecycle": "drop_report",
    "reason": "consumer_lagged"
  }
}"""

EVENTS_TERMINAL = b"""{
  "profile": "events",
  "stream": "directory",
  "kind": "directory.terminal",
  "event_id": "evt-directory-11",
  "cursor": {"stream": "directory", "sequence": 11, "token": "directory:11"},
  "resume_token": "terminal",
  "occurred_unix_ms": 1783100000123,
  "occurred_at": "2026-07-03T17:33:20.123Z",
  "subject_ref": null,
  "tenant_ref": null,
  "payload": {"reason": "client_closed"},
  "dropped_count": 0,
  "reconnect_after_ms": null,
  "terminal": true,
  "metadata": {
    "profile": "events",
    "stream": "directory",
    "carrier_owner": "daemon_sdk",
    "source": "daemon_directory_event",
    "stream_ability": "federation.subscribe_directory_v2",
    "lifecycle": "terminal",
    "reason": "client_closed"
  }
}"""

EVENTS_DEVICE_EVENT_PAGE = b"""{
  "profile": "events",
  "stream": "device",
  "item_kind": "device_event",
  "items": [
    {
      "profile": "events",
      "stream": "device",
      "kind": "device.status_changed",
      "event_id": "evt-device-8",
      "cursor": {"stream": "device", "sequence": 8, "token": "device:8"},
      "resume_token": "device:8",
      "occurred_unix_ms": 1783100000123,
      "occurred_at": "2026-07-03T17:33:20.123Z",
      "subject_ref": {"kind": "ura", "ura": "easynet:///r/example/device/dev-a", "role": "device"},
      "tenant_ref": {"kind": "realm", "realm": "example"},
      "payload": {"type": "status_changed", "state": "online"},
      "dropped_count": 0,
      "reconnect_after_ms": null,
      "terminal": false,
      "metadata": {"profile": "events", "stream": "device", "source": "daemon_device_event"}
    }
  ],
  "next_cursor": "device:9",
  "has_more": true,
  "limit": 50,
  "metadata": {"profile": "events", "source": "device_event_history"}
}"""

EVENTS_DEVICE_EVENT_PAGE_WITH_DIRECTORY_ITEM = b"""{
  "profile": "events",
  "stream": "device",
  "item_kind": "device_event",
  "items": [
    {
      "profile": "events",
      "stream": "directory",
      "kind": "directory.agent_advertised",
      "event_id": "evt-directory-8",
      "cursor": {"stream": "directory", "sequence": 8, "token": "directory:8"},
      "resume_token": "directory:8",
      "occurred_unix_ms": 1783100000123,
      "occurred_at": "2026-07-03T17:33:20.123Z",
      "subject_ref": null,
      "tenant_ref": null,
      "payload": {},
      "dropped_count": 0,
      "reconnect_after_ms": null,
      "terminal": false,
      "metadata": {"profile": "events", "stream": "directory"}
    }
  ],
  "next_cursor": null,
  "has_more": false,
  "limit": 50,
  "metadata": {"profile": "events"}
}"""

EVENTS_DEVICE_EVENT = b"""{
  "profile": "events",
  "stream": "device",
  "kind": "device.status_changed",
  "event_id": "evt-device-8",
  "cursor": {"stream": "device", "sequence": 8, "token": "device:8"},
  "resume_token": "device:8",
  "occurred_unix_ms": 1783100000123,
  "occurred_at": "2026-07-03T17:33:20.123Z",
  "subject_ref": {"kind": "ura", "ura": "easynet:///r/example/device/dev-a", "role": "device"},
  "tenant_ref": {"kind": "realm", "realm": "example"},
  "payload": {"state": "online"},
  "dropped_count": 0,
  "reconnect_after_ms": null,
  "terminal": false,
  "metadata": {"profile": "events", "stream": "device", "source": "daemon_device_event"}
}"""

EVENTS_INVOCATION_EVENT = b"""{
  "profile": "events",
  "stream": "invocation",
  "kind": "invocation.completed",
  "event_id": "evt-invocation-4",
  "cursor": {"stream": "invocation", "sequence": 4, "token": "invocation:4"},
  "resume_token": "invocation:4",
  "occurred_unix_ms": 1783100001123,
  "occurred_at": "2026-07-03T17:33:21.123Z",
  "subject_ref": {"kind": "invocation", "invocation_id": "inv-1"},
  "tenant_ref": null,
  "payload": {"terminal_state": "Completed"},
  "dropped_count": 0,
  "reconnect_after_ms": null,
  "terminal": false,
  "metadata": {"profile": "events", "stream": "invocation", "source": "daemon_invocation_event"}
}"""


def event_stream(stream: str) -> bytes:
    return (
        b'{"stream":"'
        + stream.encode("utf-8")
        + b'","stream_id":"events-1","state":"Open","resume_token":"'
        + stream.encode("utf-8")
        + b':8","metadata":{"profile":"events"}}'
    )


class MemoryEventTransport:
    def __init__(self) -> None:
        self.seen: dict[str, dict[str, object]] = {}
        self.close_calls = 0

    def _remember(self, name: str, request_json: bytes) -> None:
        self.seen[name] = json.loads(request_json.decode("utf-8"))

    def build_directory_subscription_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_directory_subscription", request_json)
        return EVENTS_DIRECTORY_SUBSCRIPTION_INVOCATION

    def build_device_subscription_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_device_subscription", request_json)
        return EVENTS_DIRECTORY_SUBSCRIPTION_INVOCATION

    def build_session_subscription_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_session_subscription", request_json)
        return EVENTS_DIRECTORY_SUBSCRIPTION_INVOCATION

    def build_invocation_subscription_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_invocation_subscription", request_json)
        return EVENTS_DIRECTORY_SUBSCRIPTION_INVOCATION

    def subscribe_directory(self, request_json: bytes) -> bytes:
        self._remember("subscribe_directory", request_json)
        return event_stream("directory")

    def subscribe_devices(self, request_json: bytes) -> bytes:
        self._remember("subscribe_devices", request_json)
        return event_stream("device")

    def subscribe_sessions(self, request_json: bytes) -> bytes:
        self._remember("subscribe_sessions", request_json)
        return event_stream("session")

    def subscribe_invocations(self, request_json: bytes) -> bytes:
        self._remember("subscribe_invocations", request_json)
        return event_stream("invocation")

    def list_device_events(self, request_json: bytes) -> bytes:
        self._remember("list_device_events", request_json)
        return EVENTS_DEVICE_EVENT_PAGE

    def project_directory_event(self, event_json: bytes) -> bytes:
        self._remember("project_directory_event", event_json)
        return EVENTS_DIRECTORY_EVENT

    def project_live_event(self, event_json: bytes) -> bytes:
        self._remember("project_live_event", event_json)
        request = self.seen["project_live_event"]
        stream = request["cursor"]["stream"]
        if stream == "device":
            return EVENTS_DEVICE_EVENT
        if stream == "invocation":
            return EVENTS_INVOCATION_EVENT
        return EVENTS_DIRECTORY_EVENT

    def project_drop_report(self, drop_json: bytes) -> bytes:
        self._remember("project_drop_report", drop_json)
        return EVENTS_DROP_REPORT

    def project_terminal(self, terminal_json: bytes) -> bytes:
        self._remember("project_terminal", terminal_json)
        return EVENTS_TERMINAL

    def close(self) -> None:
        self.close_calls += 1


class MemoryStreamTransport:
    def __init__(self, raw: bytes) -> None:
        self.raw = raw

    def recv(self, timeout: float | None = None) -> bytes:
        return self.raw

    def cancel(self, reason: str) -> bytes:
        return b'{"stream_id":"events-raw","cancelled":true,"state":"Cancelled","terminal":true}'

    def close(self) -> None:
        return None


def events_base() -> EventsCarrierBase:
    return EventsCarrierBase(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/device/dev-a",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        metadata={"request_id": "events-directory-subscribe-1"},
    )


class EventClientTests(unittest.TestCase):
    def test_builds_directory_subscription_invocation(self) -> None:
        transport = MemoryEventTransport()
        client = EventClient(transport)

        draft = client.build_directory_subscription_invocation(
            DirectoryEventQuery(
                events_base(),
                realm="example",
                agent_ura="easynet:///r/example/agent/alice.main",
                resume_cursor=EventCursor("directory", 7),
                heartbeat_interval_ms=30000,
            )
        )

        self.assertEqual(
            draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2@1.0.0",
        )
        cursor = transport.seen["build_directory_subscription"]["resume_cursor"]
        self.assertEqual(cursor["stream"], "directory")
        self.assertEqual(cursor["sequence"], 7)
        self.assertNotIn("token", cursor)

    def test_event_filter_normalizes_into_subscription_request(self) -> None:
        transport = MemoryEventTransport()
        client = EventClient(transport)

        client.build_device_subscription_invocation(
            DeviceEventQuery(
                events_base(),
                stream="device",
                filter=EventFilter(device_ura="easynet:///r/example/device/dev-a"),
            )
        )

        seen = transport.seen["build_device_subscription"]
        self.assertEqual(
            seen["filter"]["device_ura"],
            "easynet:///r/example/device/dev-a",
        )
        self.assertEqual(
            seen["device_ura"],
            "easynet:///r/example/device/dev-a",
        )

    def test_event_filter_conflict_fails_closed(self) -> None:
        client = EventClient(MemoryEventTransport())

        with self.assertRaises(SDKError) as caught:
            client.build_device_subscription_invocation(
                DeviceEventQuery(
                    events_base(),
                    stream="device",
                    device_ura="easynet:///r/example/device/dev-a",
                    filter=EventFilter(device_ura="easynet:///r/example/device/dev-b"),
                )
            )

        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

    def test_projects_frames_and_stream(self) -> None:
        client = EventClient(MemoryEventTransport())

        stream = client.subscribe_directory(
            DirectoryEventQuery(events_base())
        )
        self.assertEqual(stream.stream, "directory")
        self.assertEqual(stream.state, "Open")

        event = client.project_directory_event(
            EventProjectionInput(
                EventCursor("directory", 8),
                {
                    "type": "agent_advertised",
                    "agent_ura": "easynet:///r/example/agent/alice.main",
                    "signing_authority": "self_signed",
                    "replaced_prior": False,
                    "unix_ms": 1783100000123,
                },
            )
        )
        self.assertEqual(event.kind, "directory.agent_advertised")
        self.assertEqual(event.cursor.token, "directory:8")
        self.assertFalse(event.terminal)

        drop = client.project_drop_report(
            EventDropReportInput(
                EventCursor("directory", 10),
                occurred_unix_ms=1783100000123,
                dropped_count=4,
            )
        )
        self.assertEqual(drop.dropped_count, 4)
        self.assertEqual(drop.reconnect_after_ms, 1000)

        terminal = client.project_terminal(
            EventTerminalInput(
                EventCursor("directory", 11),
                occurred_unix_ms=1783100000123,
                reason="client_closed",
            )
        )
        self.assertTrue(terminal.terminal)
        self.assertEqual(terminal.kind, "directory.terminal")

    def test_event_stream_projects_raw_directory_payload(self) -> None:
        seen: list[EventProjectionInput] = []
        runtime_stream = StreamHandle.from_json(
            MemoryStreamTransport(
                b"""{
                  "sequence": 3,
                  "kind": "data",
                  "state": "Open",
                  "terminal": false,
                  "payload_json": {
                    "type": "agent_advertised",
                    "agent_ura": "easynet:///r/example/agent/alice.main",
                    "signing_authority": "self_signed",
                    "replaced_prior": false,
                    "unix_ms": 1783100000123
                  }
                }"""
            ),
            b'{"stream_id":"events-raw","state":"Open"}',
        )
        stream = EventStream.from_runtime_stream(
            "directory",
            runtime_stream,
            live_projector=lambda input: seen.append(input)
            or EventFrame.from_json(EVENTS_DIRECTORY_EVENT),
        )

        frame = stream.next()

        self.assertEqual(frame.kind, "directory.agent_advertised")
        self.assertEqual(seen[0].cursor.sequence, 3)
        self.assertEqual(seen[0].event["type"], "agent_advertised")

    def test_event_stream_projects_raw_device_and_invocation_payloads(self) -> None:
        seen: list[EventProjectionInput] = []

        device_stream = StreamHandle.from_json(
            MemoryStreamTransport(
                b"""{
                  "sequence": 8,
                  "kind": "data",
                  "state": "Open",
                  "terminal": false,
                  "payload_json": {
                    "sequence": 8,
                    "device_ura": "easynet:///r/example/device/dev-a",
                    "occurred_unix_ms": 1783100000123,
                    "kind": "device.status_changed",
                    "payload": {"state": "online"}
                  }
                }"""
            ),
            b'{"stream_id":"events-device-raw","state":"Open"}',
        )
        device_events = EventStream.from_runtime_stream(
            "device",
            device_stream,
            live_projector=lambda input: seen.append(input)
            or EventFrame.from_json(EVENTS_DEVICE_EVENT),
        )

        device_frame = device_events.next()

        self.assertEqual(device_frame.kind, "device.status_changed")
        self.assertEqual(seen[0].cursor.stream, "device")
        self.assertEqual(seen[0].cursor.sequence, 8)

        invocation_stream = StreamHandle.from_json(
            MemoryStreamTransport(
                b"""{
                  "sequence": 4,
                  "kind": "data",
                  "state": "Open",
                  "terminal": false,
                  "payload_json": {
                    "sequence": 4,
                    "invocation_id": "inv-1",
                    "occurred_unix_ms": 1783100001123,
                    "kind": "invocation.completed",
                    "payload": {"terminal_state": "Completed"}
                  }
                }"""
            ),
            b'{"stream_id":"events-invocation-raw","state":"Open"}',
        )
        invocation_events = EventStream.from_runtime_stream(
            "invocation",
            invocation_stream,
            live_projector=lambda input: seen.append(input)
            or EventFrame.from_json(EVENTS_INVOCATION_EVENT),
        )

        invocation_frame = invocation_events.next()

        self.assertEqual(invocation_frame.kind, "invocation.completed")
        self.assertEqual(seen[1].cursor.stream, "invocation")
        self.assertEqual(seen[1].event["invocation_id"], "inv-1")

    def test_directory_projectors_reject_session_cursor(self) -> None:
        client = EventClient(MemoryEventTransport())

        with self.assertRaises(SDKError) as caught:
            client.project_directory_event(
                EventProjectionInput(
                    EventCursor("session", 8),
                    {"type": "heartbeat", "unix_ms": 1783100000123},
                )
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn("event cursor stream mismatch", caught.exception.message)

        with self.assertRaises(SDKError) as caught:
            client.project_terminal(
                EventTerminalInput(
                    EventCursor("session", 11),
                    occurred_unix_ms=1783100000123,
                )
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn("event cursor stream mismatch", caught.exception.message)

    def test_session_subscription_requires_daemon_session_id(self) -> None:
        client = EventClient(MemoryEventTransport())

        with self.assertRaises(SDKError) as caught:
            client.subscribe_sessions(
                SessionEventQuery(
                    events_base(),
                    session_ura="easynet:///r/example/resource/daemon.browser/run-1",
                )
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn(
            "session_ura cannot be converted into daemon session_id",
            caught.exception.message,
        )

        with self.assertRaises(SDKError) as caught:
            client.subscribe_sessions(SessionEventQuery(events_base()))
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn("session_id is required", caught.exception.message)

    def test_subscribes_device_session_and_invocation_streams(self) -> None:
        transport = MemoryEventTransport()
        client = EventClient(transport)

        device_stream = client.subscribe_devices(
            DeviceEventQuery(
                events_base(),
                device_ura="easynet:///r/example/device/dev-a",
                resume_cursor=EventCursor("device", 2),
            )
        )
        self.assertEqual(device_stream.stream, "device")
        self.assertEqual(transport.seen["subscribe_devices"]["stream"], "device")

        session_stream = client.subscribe_sessions(
            SessionEventQuery(
                events_base(),
                session_id="run-1",
                resume_cursor=EventCursor("session", 4),
            )
        )
        self.assertEqual(session_stream.stream, "session")
        self.assertEqual(transport.seen["subscribe_sessions"]["session_id"], "run-1")
        self.assertEqual(
            transport.seen["subscribe_sessions"]["resume_cursor"],
            {"stream": "session", "sequence": 4},
        )

        invocation_stream = client.subscribe_invocations(
            InvocationEventQuery(events_base(), invocation_id="inv-1")
        )
        self.assertEqual(invocation_stream.stream, "invocation")
        self.assertEqual(
            transport.seen["subscribe_invocations"]["invocation_id"], "inv-1"
        )

        client.build_device_subscription_invocation(
            DeviceEventQuery(events_base())
        )
        self.assertEqual(transport.seen["build_device_subscription"]["stream"], "device")

    def test_lists_device_event_history_page(self) -> None:
        transport = MemoryEventTransport()
        client = EventClient(transport)

        page = client.list_device_events(
            EventsDeviceEventListRequest(
                events_base(),
                device_ura="easynet:///r/example/device/dev-a",
            )
        )

        self.assertEqual(page.stream, "device")
        self.assertTrue(page.has_more)
        self.assertEqual(page.next_cursor, "device:9")
        self.assertEqual(page.items[0].kind, "device.status_changed")
        self.assertEqual(transport.seen["list_device_events"]["limit"], DEFAULT_EVENT_PAGE_SIZE)

    def test_rejects_incomplete_carrier_and_invalid_cursors(self) -> None:
        client = EventClient(MemoryEventTransport())

        with self.assertRaises(Exception):
            client.build_directory_subscription_invocation(
                DirectoryEventQuery(
                    EventsCarrierBase("", "", "", "", "", {})
                )
            )
        with self.assertRaises(Exception):
            EventCursor("sessions", 1).to_json_dict()
        with self.assertRaises(Exception):
            client.subscribe_devices(
                DeviceEventQuery(
                    events_base(),
                    resume_cursor=EventCursor("session", 1),
                )
            )
        with self.assertRaises(Exception):
            client.project_drop_report(
                EventDropReportInput(
                    EventCursor("directory", 9),
                    occurred_unix_ms=1,
                    dropped_count=0,
                )
            )
        with self.assertRaises(Exception):
            client.list_device_events(
                EventsDeviceEventListRequest(events_base(), limit=MAX_EVENT_PAGE_SIZE + 1)
            )
        with self.assertRaises(Exception):
            DeviceEventPage.from_json(EVENTS_DEVICE_EVENT_PAGE_WITH_DIRECTORY_ITEM)

    def test_close_delegates_once_and_fails_closed(self) -> None:
        transport = MemoryEventTransport()
        client = EventClient(transport)

        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            client.build_directory_subscription_invocation(
                DirectoryEventQuery(events_base())
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(transport.seen, {})


if __name__ == "__main__":
    unittest.main()
