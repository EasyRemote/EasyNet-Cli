package easynet

import (
	"context"
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

func TestRuntimeEventsMapsStreamAbilitiesAndRejectsSubscribeShortcut(t *testing.T) {
	runtimeClient, err := NewRuntimeClient(RuntimeTransportFunc{})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	identityTransport := &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			eventsAbilitySubscribeDevices:     "easynet:///r/example/ability/device.dev-a.events.device.subscribe",
			eventsAbilitySubscribeSessions:    "easynet:///r/example/ability/device.dev-a.session.attach",
			eventsAbilitySubscribeInvocations: "easynet:///r/example/ability/device.dev-a.events.invocation.subscribe",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.events.device.subscribe":     "easynet:///r/example/ability/device.dev-a.events.device.subscribe@1.0.0",
			"easynet:///r/example/ability/device.dev-a.session.attach":              "easynet:///r/example/ability/device.dev-a.session.attach@1.0.0",
			"easynet:///r/example/ability/device.dev-a.events.invocation.subscribe": "easynet:///r/example/ability/device.dev-a.events.invocation.subscribe@1.0.0",
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
	if _, err := client.SubscribeDirectory(context.Background(), EventsDirectorySubscriptionRequest{EventsCarrierBase: eventsBaseForTest()}); !IsCode(err, ErrNotImplemented) {
		t.Fatalf("SubscribeDirectory error = %v, want %s", err, ErrNotImplemented)
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
