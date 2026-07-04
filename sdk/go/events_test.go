package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryEventTransport struct {
	invocation string
	stream     string
	event      string
	drop       string
	terminal   string
	page       string
	seen       map[string]map[string]any
}

func (m *memoryEventTransport) remember(name string, requestJSON []byte) {
	if m.seen == nil {
		m.seen = map[string]map[string]any{}
	}
	var decoded map[string]any
	_ = json.Unmarshal(requestJSON, &decoded)
	m.seen[name] = decoded
}

func (m *memoryEventTransport) BuildDirectorySubscriptionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_directory_subscription", requestJSON)
	return []byte(m.invocation), nil
}

func (m *memoryEventTransport) BuildDeviceSubscriptionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_device_subscription", requestJSON)
	return []byte(m.invocation), nil
}

func (m *memoryEventTransport) BuildSessionSubscriptionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_session_subscription", requestJSON)
	return []byte(m.invocation), nil
}

func (m *memoryEventTransport) BuildInvocationSubscriptionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_invocation_subscription", requestJSON)
	return []byte(m.invocation), nil
}

func (m *memoryEventTransport) SubscribeDirectory(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("subscribe_directory", requestJSON)
	return []byte(m.stream), nil
}

func (m *memoryEventTransport) SubscribeDevices(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("subscribe_devices", requestJSON)
	return []byte(eventStreamJSON("device")), nil
}

func (m *memoryEventTransport) SubscribeSessions(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("subscribe_sessions", requestJSON)
	return []byte(eventStreamJSON("session")), nil
}

func (m *memoryEventTransport) SubscribeInvocations(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("subscribe_invocations", requestJSON)
	return []byte(eventStreamJSON("invocation")), nil
}

func (m *memoryEventTransport) ListDeviceEvents(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("list_device_events", requestJSON)
	return []byte(m.page), nil
}

func (m *memoryEventTransport) ProjectDirectoryEvent(_ context.Context, eventJSON []byte) ([]byte, error) {
	m.remember("project_directory_event", eventJSON)
	return []byte(m.event), nil
}

func (m *memoryEventTransport) ProjectDropReport(_ context.Context, dropJSON []byte) ([]byte, error) {
	m.remember("project_drop_report", dropJSON)
	return []byte(m.drop), nil
}

func (m *memoryEventTransport) ProjectTerminal(_ context.Context, terminalJSON []byte) ([]byte, error) {
	m.remember("project_terminal", terminalJSON)
	return []byte(m.terminal), nil
}

func eventsBaseForTest() EventsCarrierBase {
	return EventsCarrierBase{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		Metadata:          map[string]any{"request_id": "events-directory-subscribe-1"},
	}
}

func eventStreamJSON(stream string) string {
	return `{"stream":"` + stream + `","stream_id":"events-1","state":"Open","resume_token":"` + stream + `:8","metadata":{"profile":"events"}}`
}

func TestEventsBuildsDirectorySubscriptionInvocation(t *testing.T) {
	cursor, err := NewEventCursor("directory", 7)
	if err != nil {
		t.Fatal(err)
	}
	transport := &memoryEventTransport{invocation: eventsDirectorySubscriptionInvocationJSON}
	client, err := NewEventClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	draft, err := client.BuildDirectorySubscriptionInvocation(context.Background(), EventsDirectorySubscriptionRequest{
		EventsCarrierBase:   eventsBaseForTest(),
		Realm:               "example",
		AgentURA:            "easynet:///r/example/agent/alice.main",
		ResumeCursor:        &cursor,
		HeartbeatIntervalMS: 30000,
	})
	if err != nil {
		t.Fatal(err)
	}

	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2@1.0.0" {
		t.Fatalf("descriptor = %q", draft.DescriptorRef())
	}
	seenCursor := transport.seen["build_directory_subscription"]["resume_cursor"].(map[string]any)
	if _, ok := seenCursor["token"]; ok {
		t.Fatalf("subscription cursor must not include token: %#v", seenCursor)
	}
	if seenCursor["stream"] != "directory" || seenCursor["sequence"].(float64) != 7 {
		t.Fatalf("unexpected resume cursor: %#v", seenCursor)
	}
}

func TestEventsProjectsFramesAndStream(t *testing.T) {
	cursor, err := NewEventCursor("directory", 8)
	if err != nil {
		t.Fatal(err)
	}
	transport := &memoryEventTransport{
		stream:   eventStreamJSON("directory"),
		event:    eventsDirectoryEventJSON,
		drop:     eventsDropReportJSON,
		terminal: eventsTerminalJSON,
	}
	client, err := NewEventClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	stream, err := client.SubscribeDirectory(context.Background(), EventsDirectorySubscriptionRequest{EventsCarrierBase: eventsBaseForTest()})
	if err != nil {
		t.Fatal(err)
	}
	if stream.Stream != "directory" || stream.State != "Open" {
		t.Fatalf("unexpected stream: %#v", stream)
	}

	event, err := client.ProjectDirectoryEvent(context.Background(), EventProjectionInput{
		Cursor: cursor,
		Event: map[string]any{
			"type":              "agent_advertised",
			"agent_ura":         "easynet:///r/example/agent/alice.main",
			"signing_authority": "self_signed",
			"replaced_prior":    false,
			"unix_ms":           1783100000123,
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	if event.Kind != "directory.agent_advertised" || event.Cursor.Token != "directory:8" || event.Terminal {
		t.Fatalf("unexpected directory event: %#v", event)
	}

	dropCursor, _ := NewEventCursor("directory", 10)
	drop, err := client.ProjectDropReport(context.Background(), EventDropReportInput{
		Cursor:         dropCursor,
		OccurredUnixMS: 1783100000123,
		DroppedCount:   4,
	})
	if err != nil {
		t.Fatal(err)
	}
	if drop.DroppedCount != 4 || drop.ReconnectAfterMS == nil || *drop.ReconnectAfterMS != 1000 {
		t.Fatalf("unexpected drop report: %#v", drop)
	}

	terminalCursor, _ := NewEventCursor("directory", 11)
	terminal, err := client.ProjectTerminal(context.Background(), EventTerminalInput{
		Cursor:         terminalCursor,
		OccurredUnixMS: 1783100000123,
		Reason:         "client_closed",
	})
	if err != nil {
		t.Fatal(err)
	}
	if !terminal.Terminal || terminal.Kind != "directory.terminal" {
		t.Fatalf("unexpected terminal frame: %#v", terminal)
	}
}

func TestEventsSubscribesDeviceSessionAndInvocationStreams(t *testing.T) {
	transport := &memoryEventTransport{invocation: eventsDirectorySubscriptionInvocationJSON}
	client, err := NewEventClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	deviceCursor, err := NewEventCursor("device", 2)
	if err != nil {
		t.Fatal(err)
	}
	deviceStream, err := client.SubscribeDevices(context.Background(), EventsDeviceSubscriptionRequest{
		EventsCarrierBase: eventsBaseForTest(),
		DeviceURA:         "easynet:///r/example/device/dev-a",
		ResumeCursor:      &deviceCursor,
	})
	if err != nil {
		t.Fatal(err)
	}
	if deviceStream.Stream != "device" {
		t.Fatalf("unexpected device stream: %#v", deviceStream)
	}
	if transport.seen["subscribe_devices"]["stream"] != "device" {
		t.Fatalf("device subscription stream not normalized: %#v", transport.seen["subscribe_devices"])
	}

	sessionStream, err := client.SubscribeSessions(context.Background(), EventsSessionSubscriptionRequest{
		EventsCarrierBase: eventsBaseForTest(),
		SessionURA:        "easynet:///r/example/session/sess-1",
	})
	if err != nil {
		t.Fatal(err)
	}
	if sessionStream.Stream != "session" || transport.seen["subscribe_sessions"]["session_ura"] == "" {
		t.Fatalf("unexpected session stream/request: stream=%#v request=%#v", sessionStream, transport.seen["subscribe_sessions"])
	}

	invocationStream, err := client.SubscribeInvocations(context.Background(), EventsInvocationSubscriptionRequest{
		EventsCarrierBase: eventsBaseForTest(),
		InvocationID:      "inv-1",
	})
	if err != nil {
		t.Fatal(err)
	}
	if invocationStream.Stream != "invocation" || transport.seen["subscribe_invocations"]["invocation_id"] != "inv-1" {
		t.Fatalf("unexpected invocation stream/request: stream=%#v request=%#v", invocationStream, transport.seen["subscribe_invocations"])
	}

	if _, err := client.BuildDeviceSubscriptionInvocation(context.Background(), EventsDeviceSubscriptionRequest{EventsCarrierBase: eventsBaseForTest()}); err != nil {
		t.Fatal(err)
	}
	if transport.seen["build_device_subscription"]["stream"] != "device" {
		t.Fatalf("device invocation stream not normalized: %#v", transport.seen["build_device_subscription"])
	}
}

func TestEventsListsDeviceEventHistoryPage(t *testing.T) {
	transport := &memoryEventTransport{page: eventsDeviceEventPageJSON}
	client, err := NewEventClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	page, err := client.ListDeviceEvents(context.Background(), EventsDeviceEventListRequest{
		EventsCarrierBase: eventsBaseForTest(),
		DeviceURA:         "easynet:///r/example/device/dev-a",
	})
	if err != nil {
		t.Fatal(err)
	}
	if page.Stream != "device" || !page.HasMore || page.NextCursor == nil || *page.NextCursor != "device:9" {
		t.Fatalf("unexpected device event page: %#v", page)
	}
	if len(page.Items) != 1 || page.Items[0].Kind != "device.status_changed" {
		t.Fatalf("unexpected page items: %#v", page.Items)
	}
	if transport.seen["list_device_events"]["limit"].(float64) != DefaultEventPageSize {
		t.Fatalf("default limit not applied: %#v", transport.seen["list_device_events"])
	}
}

func TestEventsRejectsIncompleteCarrierAndInvalidCursors(t *testing.T) {
	transport := &memoryEventTransport{invocation: eventsDirectorySubscriptionInvocationJSON, drop: eventsDropReportJSON, page: eventsDeviceEventPageJSON}
	client, err := NewEventClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	if _, err := client.BuildDirectorySubscriptionInvocation(context.Background(), EventsDirectorySubscriptionRequest{}); err == nil {
		t.Fatal("expected incomplete carrier rejection")
	}
	if _, err := NewEventCursor("sessions", 1); err == nil {
		t.Fatal("expected unsupported stream rejection")
	}
	sessionCursor, _ := NewEventCursor("session", 1)
	if _, err := client.SubscribeDevices(context.Background(), EventsDeviceSubscriptionRequest{
		EventsCarrierBase: eventsBaseForTest(),
		ResumeCursor:      &sessionCursor,
	}); err == nil {
		t.Fatal("expected cursor stream mismatch rejection")
	}
	cursor, _ := NewEventCursor("directory", 9)
	if _, err := client.ProjectDropReport(context.Background(), EventDropReportInput{Cursor: cursor, OccurredUnixMS: 1, DroppedCount: 0}); err == nil {
		t.Fatal("expected zero dropped_count rejection")
	}
	if _, err := client.ListDeviceEvents(context.Background(), EventsDeviceEventListRequest{
		EventsCarrierBase: eventsBaseForTest(),
		Limit:             MaxEventPageSize + 1,
	}); err == nil {
		t.Fatal("expected event page limit rejection")
	}
	if _, err := NewDeviceEventPageFromJSON([]byte(eventsDeviceEventPageWithDirectoryItemJSON)); err == nil {
		t.Fatal("expected device event page item stream mismatch rejection")
	}
}

const eventsDirectorySubscriptionInvocationJSON = `{
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
}`

const eventsDirectoryEventJSON = `{
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
}`

const eventsDropReportJSON = `{
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
}`

const eventsTerminalJSON = `{
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
}`

const eventsDeviceEventPageJSON = `{
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
}`

const eventsDeviceEventPageWithDirectoryItemJSON = `{
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
}`
