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
			eventsAbilitySubscribeDevices: "easynet:///r/example/ability/device.dev-a.events.subscribe_devices",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.events.subscribe_devices": "easynet:///r/example/ability/device.dev-a.events.subscribe_devices@1.0.0",
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
	if _, err := client.SubscribeDirectory(context.Background(), EventsDirectorySubscriptionRequest{EventsCarrierBase: eventsBaseForTest()}); !IsCode(err, ErrNotImplemented) {
		t.Fatalf("SubscribeDirectory error = %v, want %s", err, ErrNotImplemented)
	}
}
