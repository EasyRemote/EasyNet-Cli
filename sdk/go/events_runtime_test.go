package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

func TestRuntimeEventsBuildsDirectorySubscriptionInvocation(t *testing.T) {
	runtimeClient, err := NewRuntimeClient(RuntimeTransportFunc{})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	identityTransport := &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			eventsAbilitySubscribeDirectory: "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2": "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2@1.0.0",
		},
	}
	identityClient, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	client, err := NewRuntimeEventClient(runtimeClient, identityClient)
	if err != nil {
		t.Fatalf("NewRuntimeEventClient: %v", err)
	}
	cursor, err := NewEventCursor("directory", 7)
	if err != nil {
		t.Fatalf("NewEventCursor: %v", err)
	}

	draft, err := client.BuildDirectorySubscriptionInvocation(context.Background(), EventsDirectorySubscriptionRequest{
		EventsCarrierBase:   eventsBaseForTest(),
		Realm:               "example",
		AgentURA:            "easynet:///r/example/agent/alice.main",
		ResumeCursor:        &cursor,
		HeartbeatIntervalMS: 30000,
	})
	if err != nil {
		t.Fatalf("BuildDirectorySubscriptionInvocation: %v", err)
	}

	if got, want := draft.DescriptorRef(), "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2@1.0.0"; got != want {
		t.Fatalf("descriptor_ref = %q, want %q", got, want)
	}
	args, ok := draft.JSONArgs().(map[string]any)
	if !ok {
		t.Fatalf("args = %#v, want map", draft.JSONArgs())
	}
	if args["daemon_ability"] != eventsAbilitySubscribeDirectory ||
		args["stream"] != "directory" ||
		args["resume_cursor"] != "directory:7" ||
		args["heartbeat_interval_ms"].(float64) != 30000 {
		t.Fatalf("unexpected events args: %#v", args)
	}
	metadata := draft.Metadata()
	if metadata["profile"] != eventsProfile ||
		metadata["system_ability"] != eventsAbilitySubscribeDirectory ||
		metadata["carrier_owner"] != "daemon_sdk" ||
		metadata["request_id"] != "events-directory-subscribe-1" {
		t.Fatalf("unexpected metadata: %#v", metadata)
	}
	if len(identityTransport.seenBuildURA) != 1 ||
		identityTransport.seenBuildURA[0]["ability_name"] != eventsAbilitySubscribeDirectory {
		t.Fatalf("identity ability lookup was not delegated: %#v", identityTransport.seenBuildURA)
	}
}

func TestRuntimeEventsMapsStreamAbilitiesAndOpensRuntimeStream(t *testing.T) {
	runtimeTransport := &compatibilityRuntimeInvokeTransport{
		streamTransport: StreamTransportFunc{
			RecvFunc: func(ctx context.Context) ([]byte, error) {
				return []byte(`{
					"sequence": 1,
					"kind": "data",
					"state": "Open",
					"terminal": false,
					"payload_content_type": "application/json",
					"payload_json": {
						"profile": "events",
						"stream": "directory",
						"kind": "directory_delta",
						"event_id": "evt-directory-1",
						"cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
						"resume_token": "directory:1",
						"occurred_unix_ms": 1000,
						"occurred_at": "1970-01-01T00:00:01Z",
						"subject_ref": {"kind": "directory"},
						"tenant_ref": null,
						"payload": {"op": "upsert"},
						"dropped_count": 0,
						"reconnect_after_ms": null,
						"terminal": false,
						"metadata": {"source": "runtime"}
					}
				}`), nil
			},
			CancelFunc: func(ctx context.Context, reason string) ([]byte, error) {
				if reason != "done" {
					t.Fatalf("cancel reason = %q, want done", reason)
				}
				return []byte(`{"stream_id":"runtime-stream-1","cancelled":true,"state":"Cancelled","terminal":true}`), nil
			},
		},
	}
	runtimeClient, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	identityTransport := &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			eventsAbilitySubscribeDirectory:   "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2",
			eventsAbilitySubscribeDevices:     "easynet:///r/example/ability/device.dev-a.events.device.subscribe",
			eventsAbilitySubscribeSessions:    "easynet:///r/example/ability/device.dev-a.session.attach",
			eventsAbilitySubscribeInvocations: "easynet:///r/example/ability/device.dev-a.events.invocation.subscribe",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2": "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2@1.0.0",
			"easynet:///r/example/ability/device.dev-a.events.device.subscribe":           "easynet:///r/example/ability/device.dev-a.events.device.subscribe@1.0.0",
			"easynet:///r/example/ability/device.dev-a.session.attach":                    "easynet:///r/example/ability/device.dev-a.session.attach@1.0.0",
			"easynet:///r/example/ability/device.dev-a.events.invocation.subscribe":       "easynet:///r/example/ability/device.dev-a.events.invocation.subscribe@1.0.0",
		},
	}
	identityClient, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	client, err := NewRuntimeEventClient(runtimeClient, identityClient)
	if err != nil {
		t.Fatalf("NewRuntimeEventClient: %v", err)
	}

	draft, err := client.BuildDeviceSubscriptionInvocation(context.Background(), EventsDeviceSubscriptionRequest{
		EventsCarrierBase: eventsBaseForTest(),
		DeviceURA:         "easynet:///r/example/device/dev-a",
	})
	if err != nil {
		t.Fatalf("BuildDeviceSubscriptionInvocation: %v", err)
	}
	args := draft.JSONArgs().(map[string]any)
	if args["daemon_ability"] != eventsAbilitySubscribeDevices || args["stream"] != "device" {
		t.Fatalf("unexpected device event args: %#v", args)
	}

	sessionCursor, _ := NewEventCursor("session", 4)
	sessionDraft, err := client.BuildSessionSubscriptionInvocation(context.Background(), EventsSessionSubscriptionRequest{
		EventsCarrierBase: eventsBaseForTest(),
		SessionID:         "run-1",
		ResumeCursor:      &sessionCursor,
	})
	if err != nil {
		t.Fatalf("BuildSessionSubscriptionInvocation: %v", err)
	}
	sessionArgs := sessionDraft.JSONArgs().(map[string]any)
	if sessionDraft.Metadata()["system_ability"] != eventsAbilitySubscribeSessions ||
		sessionDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.session.attach@1.0.0" ||
		sessionArgs["session_id"] != "run-1" ||
		sessionArgs["since_seq"].(float64) != 4 ||
		sessionArgs["stream"] != nil ||
		sessionArgs["daemon_ability"] != nil {
		t.Fatalf("unexpected session event args: draft=%#v args=%#v", sessionDraft, sessionArgs)
	}

	invocationDraft, err := client.BuildInvocationSubscriptionInvocation(context.Background(), EventsInvocationSubscriptionRequest{
		EventsCarrierBase: eventsBaseForTest(),
		InvocationID:      "inv-1",
	})
	if err != nil {
		t.Fatalf("BuildInvocationSubscriptionInvocation: %v", err)
	}
	invocationArgs := invocationDraft.JSONArgs().(map[string]any)
	if invocationDraft.Metadata()["system_ability"] != eventsAbilitySubscribeInvocations ||
		invocationArgs["daemon_ability"] != eventsAbilitySubscribeInvocations ||
		invocationArgs["stream"] != "invocation" ||
		invocationArgs["invocation_id"] != "inv-1" {
		t.Fatalf("unexpected invocation event args: draft=%#v args=%#v", invocationDraft, invocationArgs)
	}

	stream, err := client.SubscribeDirectory(context.Background(), EventsDirectorySubscriptionRequest{EventsCarrierBase: eventsBaseForTest()})
	if err != nil {
		t.Fatalf("SubscribeDirectory: %v", err)
	}
	if stream.Stream != string(EventStreamDirectory) || stream.StreamID != "runtime-stream-1" || stream.State != string(StreamOpen) {
		t.Fatalf("unexpected event stream: %#v", stream)
	}
	if !runtimeTransport.openStreamCalled || runtimeTransport.seenStreamDraft["descriptor_ref"] == "" {
		t.Fatalf("runtime stream was not opened with a complete draft: %#v", runtimeTransport.seenStreamDraft)
	}
	frame, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("EventStream.Next: %v", err)
	}
	if frame.Stream != string(EventStreamDirectory) || frame.EventID != "evt-directory-1" {
		t.Fatalf("unexpected event frame: %#v", frame)
	}
	cancel, err := stream.Cancel(context.Background(), "done")
	if err != nil {
		t.Fatalf("EventStream.Cancel: %v", err)
	}
	if !cancel.Cancelled() || cancel.State() != StreamCancelled {
		t.Fatalf("unexpected cancel result: %#v", cancel)
	}
}

func TestRuntimeEventsProjectsRawDirectoryStreamPayload(t *testing.T) {
	runtimeTransport := &compatibilityRuntimeInvokeTransport{
		streamTransport: StreamTransportFunc{
			RecvFunc: func(ctx context.Context) ([]byte, error) {
				return []byte(`{
					"sequence": 3,
					"kind": "data",
					"state": "Open",
					"terminal": false,
					"payload_content_type": "application/json",
					"payload_json": {
						"type": "agent_advertised",
						"agent_ura": "easynet:///r/example/agent/alice.main",
						"signing_authority": "self_signed",
						"replaced_prior": false,
						"unix_ms": 1783100000123
					}
				}`), nil
			},
		},
	}
	runtimeClient, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	identityTransport := &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			eventsAbilitySubscribeDirectory: "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2": "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2@1.0.0",
		},
	}
	identityClient, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	var seenProjection map[string]any
	provider := EventTransportFunc{
		ProjectDirectoryEventFunc: func(ctx context.Context, eventJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(eventJSON, &seenProjection); err != nil {
				return nil, err
			}
			return []byte(eventsRuntimeDirectoryFrameJSON), nil
		},
	}
	client, err := NewRuntimeEventClientWithProjectionProvider(runtimeClient, identityClient, provider)
	if err != nil {
		t.Fatalf("NewRuntimeEventClientWithProjectionProvider: %v", err)
	}

	stream, err := client.SubscribeDirectory(context.Background(), EventsDirectorySubscriptionRequest{EventsCarrierBase: eventsBaseForTest()})
	if err != nil {
		t.Fatalf("SubscribeDirectory: %v", err)
	}
	frame, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("EventStream.Next raw DirectoryEvent: %v", err)
	}

	if frame.EventID != "evt-directory-runtime" {
		t.Fatalf("projected frame = %#v", frame)
	}
	cursor := seenProjection["cursor"].(map[string]any)
	event := seenProjection["event"].(map[string]any)
	if cursor["stream"] != "directory" || cursor["sequence"].(float64) != 3 ||
		event["type"] != "agent_advertised" {
		t.Fatalf("projection input = %#v", seenProjection)
	}
}

func TestRuntimeEventsListsDeviceHistoryThroughRuntime(t *testing.T) {
	identityTransport := &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			eventsAbilityDeviceHistory: "easynet:///r/example/ability/device.dev-a.events.device.history",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.events.device.history": "easynet:///r/example/ability/device.dev-a.events.device.history@1.0.0",
		},
	}
	identityClient, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: eventsRuntimeDeviceHistoryRawJSON}
	runtimeClient, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeEventClient(runtimeClient, identityClient)
	if err != nil {
		t.Fatalf("NewRuntimeEventClient: %v", err)
	}

	page, err := client.ListDeviceEvents(context.Background(), EventsDeviceEventListRequest{
		EventsCarrierBase: eventsBaseForTest(),
		DeviceURA:         "easynet:///r/example/device/dev-a",
		Limit:             1,
	})
	if err != nil {
		t.Fatalf("ListDeviceEvents: %v", err)
	}
	if page.Stream != "device" || page.ItemKind != "device_event" || len(page.Items) != 1 ||
		page.Items[0].Kind != "device.status_changed" || page.NextCursor == nil || *page.NextCursor != "device:1" {
		t.Fatalf("device history page = %#v", page)
	}
	args := runtimeTransport.seenDraft["args"].(map[string]any)
	if args["daemon_ability"] != eventsAbilityDeviceHistory ||
		args["stream"] != "device" ||
		args["device_ura"] != "easynet:///r/example/device/dev-a" ||
		args["limit"].(float64) != 1 {
		t.Fatalf("runtime device history args = %#v", args)
	}
	if runtimeTransport.seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.events.device.history@1.0.0" {
		t.Fatalf("descriptor_ref = %#v", runtimeTransport.seenDraft["descriptor_ref"])
	}
	if len(identityTransport.seenBuildURA) != 1 ||
		identityTransport.seenBuildURA[0]["ability_name"] != eventsAbilityDeviceHistory {
		t.Fatalf("identity lookup not delegated: %#v", identityTransport.seenBuildURA)
	}
}

func TestRuntimeEventsDelegatesFrameProjectionsToProvider(t *testing.T) {
	runtimeClient, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	identityClient, err := NewIdentityClient(&compatibilityRuntimeIdentityTransport{})
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	seen := map[string]map[string]any{}
	provider := EventTransportFunc{
		ProjectDirectoryEventFunc: func(ctx context.Context, eventJSON []byte) ([]byte, error) {
			var request map[string]any
			if err := json.Unmarshal(eventJSON, &request); err != nil {
				return nil, err
			}
			seen["directory"] = request
			return []byte(eventsRuntimeDirectoryFrameJSON), nil
		},
		ProjectDropReportFunc: func(ctx context.Context, dropJSON []byte) ([]byte, error) {
			var request map[string]any
			if err := json.Unmarshal(dropJSON, &request); err != nil {
				return nil, err
			}
			seen["drop"] = request
			return []byte(eventsRuntimeDropFrameJSON), nil
		},
		ProjectTerminalFunc: func(ctx context.Context, terminalJSON []byte) ([]byte, error) {
			var request map[string]any
			if err := json.Unmarshal(terminalJSON, &request); err != nil {
				return nil, err
			}
			seen["terminal"] = request
			return []byte(eventsRuntimeTerminalFrameJSON), nil
		},
	}
	client, err := NewRuntimeEventClientWithProjectionProvider(runtimeClient, identityClient, provider)
	if err != nil {
		t.Fatalf("NewRuntimeEventClientWithProjectionProvider: %v", err)
	}
	cursor, err := NewEventCursor("directory", 1)
	if err != nil {
		t.Fatalf("NewEventCursor: %v", err)
	}
	reconnectAfterMS := 1000

	directory, err := client.ProjectDirectoryEvent(context.Background(), EventProjectionInput{
		Cursor: cursor,
		Event:  map[string]any{"kind": "directory_delta"},
	})
	if err != nil {
		t.Fatalf("ProjectDirectoryEvent: %v", err)
	}
	drop, err := client.ProjectDropReport(context.Background(), EventDropReportInput{
		Cursor:           cursor,
		OccurredUnixMS:   1001,
		DroppedCount:     3,
		ReconnectAfterMS: &reconnectAfterMS,
	})
	if err != nil {
		t.Fatalf("ProjectDropReport: %v", err)
	}
	terminal, err := client.ProjectTerminal(context.Background(), EventTerminalInput{
		Cursor:           cursor,
		OccurredUnixMS:   1002,
		ReconnectAfterMS: &reconnectAfterMS,
		Reason:           "closed",
	})
	if err != nil {
		t.Fatalf("ProjectTerminal: %v", err)
	}

	if directory.EventID != "evt-directory-runtime" || drop.DroppedCount != 3 || !terminal.Terminal {
		t.Fatalf("unexpected frames: directory=%#v drop=%#v terminal=%#v", directory, drop, terminal)
	}
	if seen["directory"]["event"] == nil || seen["drop"]["dropped_count"].(float64) != 3 || seen["terminal"]["reason"] != "closed" {
		t.Fatalf("provider requests = %#v", seen)
	}
}

func TestRuntimeEventsProjectionMethodsFailClosedWithoutProvider(t *testing.T) {
	runtimeClient, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	identityClient, err := NewIdentityClient(&compatibilityRuntimeIdentityTransport{})
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	client, err := NewRuntimeEventClient(runtimeClient, identityClient)
	if err != nil {
		t.Fatalf("NewRuntimeEventClient: %v", err)
	}
	cursor, err := NewEventCursor("directory", 1)
	if err != nil {
		t.Fatalf("NewEventCursor: %v", err)
	}

	if _, err := client.ProjectDirectoryEvent(context.Background(), EventProjectionInput{Cursor: cursor, Event: map[string]any{"kind": "directory_delta"}}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("ProjectDirectoryEvent error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := client.ProjectDropReport(context.Background(), EventDropReportInput{Cursor: cursor, OccurredUnixMS: 1, DroppedCount: 1}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("ProjectDropReport error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := client.ProjectTerminal(context.Background(), EventTerminalInput{Cursor: cursor, OccurredUnixMS: 1}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("ProjectTerminal error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestRuntimeEventsRejectsDeviceHistoryForDifferentDevice(t *testing.T) {
	identityTransport := &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			eventsAbilityDeviceHistory: "easynet:///r/example/ability/device.dev-a.events.device.history",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.events.device.history": "easynet:///r/example/ability/device.dev-a.events.device.history@1.0.0",
		},
	}
	identityClient, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeClient, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: eventsRuntimeDeviceHistoryDifferentDeviceJSON})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeEventClient(runtimeClient, identityClient)
	if err != nil {
		t.Fatalf("NewRuntimeEventClient: %v", err)
	}

	_, err = client.ListDeviceEvents(context.Background(), EventsDeviceEventListRequest{
		EventsCarrierBase: eventsBaseForTest(),
		DeviceURA:         "easynet:///r/example/device/dev-a",
		Limit:             1,
	})
	if err == nil {
		t.Fatal("expected mismatched device event history rejection")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error = %v, want %s", err, ErrInvalidArgument)
	}
}

const eventsRuntimeDeviceHistoryRawJSON = `{
  "events": [
    {
      "sequence": 8,
      "device_ura": "easynet:///r/example/device/dev-a",
      "realm": "example",
      "occurred_unix_ms": 1783100000123,
      "kind": "device.status_changed",
      "payload": {"state": "online"}
    },
    {
      "sequence": 9,
      "device_ura": "easynet:///r/example/device/dev-a",
      "realm": "example",
      "occurred_unix_ms": 1783100001123,
      "kind": "device.status_changed",
      "payload": {"state": "offline"}
    }
  ]
}`

const eventsRuntimeDirectoryFrameJSON = `{
  "profile": "events",
  "stream": "directory",
  "kind": "directory_delta",
  "event_id": "evt-directory-runtime",
  "cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
  "resume_token": "directory:1",
  "occurred_unix_ms": 1000,
  "occurred_at": "1970-01-01T00:00:01Z",
  "subject_ref": {"kind": "directory"},
  "tenant_ref": null,
  "payload": {"op": "upsert"},
  "dropped_count": 0,
  "reconnect_after_ms": null,
  "terminal": false,
  "metadata": {"source": "runtime_projection_provider"}
}`

const eventsRuntimeDropFrameJSON = `{
  "profile": "events",
  "stream": "directory",
  "kind": "directory_drop_report",
  "event_id": "evt-directory-drop-runtime",
  "cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
  "resume_token": "directory:1",
  "occurred_unix_ms": 1001,
  "occurred_at": "1970-01-01T00:00:01.001Z",
  "subject_ref": {"kind": "directory"},
  "tenant_ref": null,
  "payload": null,
  "dropped_count": 3,
  "reconnect_after_ms": 1000,
  "terminal": false,
  "metadata": {"source": "runtime_projection_provider"}
}`

const eventsRuntimeTerminalFrameJSON = `{
  "profile": "events",
  "stream": "directory",
  "kind": "directory_terminal",
  "event_id": "evt-directory-terminal-runtime",
  "cursor": {"stream": "directory", "sequence": 1, "token": "directory:1"},
  "resume_token": "directory:1",
  "occurred_unix_ms": 1002,
  "occurred_at": "1970-01-01T00:00:01.002Z",
  "subject_ref": {"kind": "directory"},
  "tenant_ref": null,
  "payload": {"reason": "closed"},
  "dropped_count": 0,
  "reconnect_after_ms": 1000,
  "terminal": true,
  "metadata": {"source": "runtime_projection_provider"}
}`

const eventsRuntimeDeviceHistoryDifferentDeviceJSON = `{
  "events": [
    {
      "sequence": 8,
      "device_ura": "easynet:///r/example/device/dev-b",
      "realm": "example",
      "occurred_unix_ms": 1783100000123,
      "kind": "device.status_changed",
      "payload": {"state": "online"}
    }
  ]
}`
