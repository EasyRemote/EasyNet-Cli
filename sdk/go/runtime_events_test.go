package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

func TestRuntimeEventClientReadsBoundedTypedPage(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		HandleEventsFunc: func(ctx context.Context, handleID uint64) ([]byte, error) {
			if handleID != 7 {
				t.Fatalf("handle id = %d", handleID)
			}
			return []byte(`{
				"handle_id":7,
				"state":"Completed",
				"terminal":true,
				"events":[
					{"sequence":1,"kind":"submitted","state":"Submitted","terminal":false},
					{"sequence":2,"kind":"running","state":"Running","terminal":false},
					{"sequence":3,"kind":"completed","state":"Completed","terminal":true,"result":{"ok":true}}
				],
				"result":{"ok":true}
			}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	provider, err := NewRuntimeHandleEventProvider(runtime)
	if err != nil {
		t.Fatalf("NewRuntimeHandleEventProvider: %v", err)
	}
	client, err := NewRuntimeEventClient(provider)
	if err != nil {
		t.Fatalf("NewRuntimeEventClient: %v", err)
	}
	handle, err := NewInvocationHandleFromJSON([]byte(`{"handle_id":7,"state":"Submitted","terminal":false,"events":[],"result":null}`))
	if err != nil {
		t.Fatalf("NewInvocationHandleFromJSON: %v", err)
	}

	page, err := client.Read(context.Background(), RuntimeEventReadRequest{
		Handle: handle,
		Cursor: &RuntimeEventCursor{Sequence: 1},
		Limit:  1,
	})
	if err != nil {
		t.Fatalf("Read: %v", err)
	}
	if len(page.Events) != 1 || page.Events[0].Sequence != 2 || page.Cursor.Sequence != 2 {
		t.Fatalf("unexpected page: %#v", page)
	}
	if page.State != RuntimeEventStreamTerminal || !page.Terminal {
		t.Fatalf("terminal state not projected: %#v", page)
	}
}

func TestRuntimeEventClientRejectsUnboundedLimit(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		HandleEventsFunc: func(context.Context, uint64) ([]byte, error) {
			t.Fatal("transport must not be called for invalid limit")
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	provider, err := NewRuntimeHandleEventProvider(runtime)
	if err != nil {
		t.Fatalf("NewRuntimeHandleEventProvider: %v", err)
	}
	handle, err := NewInvocationHandleFromJSON([]byte(`{"handle_id":7,"state":"Submitted","terminal":false,"events":[],"result":null}`))
	if err != nil {
		t.Fatalf("NewInvocationHandleFromJSON: %v", err)
	}

	_, err = provider.ReadEvents(context.Background(), RuntimeEventReadRequest{
		Handle: handle,
		Limit:  MaxRuntimeEventPageLimit + 1,
	})
	if err == nil {
		t.Fatal("ReadEvents accepted unbounded limit")
	}
}

func TestRuntimeEventSubscriptionProviderBuildsDeviceDraft(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	ability, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}
	provider, err := NewRuntimeAbilityEventSubscriptionProvider(ability)
	if err != nil {
		t.Fatalf("NewRuntimeAbilityEventSubscriptionProvider: %v", err)
	}
	client, err := NewRuntimeEventSubscriptionClient(provider)
	if err != nil {
		t.Fatalf("NewRuntimeEventSubscriptionClient: %v", err)
	}

	draft, err := client.Build(context.Background(), RuntimeEventSubscriptionRequest{
		Call:                runtimeEventTestCall(),
		Stream:              RuntimeEventStreamDevice,
		Realm:               "example",
		OwnerURA:            "easynet:///r/example/user/alice",
		DeviceURA:           "easynet:///r/example/device/laptop",
		ResumeCursor:        &RuntimeEventSubscriptionCursor{Stream: "device", Sequence: 42},
		HeartbeatIntervalMS: 30000,
	})
	if err != nil {
		t.Fatalf("Build: %v", err)
	}
	if got, want := draft.DescriptorRef(), "easynet:///r/example/ability/hub.events.device.subscribe@1.0.0"; got != want {
		t.Fatalf("descriptor_ref = %q, want %q", got, want)
	}
	if got := draft.Metadata()["sdk_profile"]; got != "runtime_events" {
		t.Fatalf("sdk_profile metadata = %#v", got)
	}
	if got := draft.Metadata()["system_ability"]; got != "events.device.subscribe" {
		t.Fatalf("system_ability metadata = %#v", got)
	}
	args := runtimeEventDraftArgs(t, draft)
	for key, want := range map[string]any{
		"stream":                "device",
		"daemon_ability":        "events.device.subscribe",
		"realm":                 "example",
		"owner_ura":             "easynet:///r/example/user/alice",
		"device_ura":            "easynet:///r/example/device/laptop",
		"heartbeat_interval_ms": float64(30000),
		"resume_cursor":         "device:42",
	} {
		if got := args[key]; got != want {
			t.Fatalf("arg %s = %#v, want %#v", key, got, want)
		}
	}
}

func TestRuntimeEventSubscriptionProviderBuildsSessionDraftWithSinceSequence(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	ability, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}
	provider, err := NewRuntimeAbilityEventSubscriptionProvider(ability)
	if err != nil {
		t.Fatalf("NewRuntimeAbilityEventSubscriptionProvider: %v", err)
	}

	draft, err := provider.BuildSubscription(context.Background(), RuntimeEventSubscriptionRequest{
		Call:         runtimeEventTestCall(),
		Stream:       RuntimeEventStreamSession,
		SessionID:    "session-a",
		ResumeCursor: &RuntimeEventSubscriptionCursor{Stream: "session", Sequence: 42},
	})
	if err != nil {
		t.Fatalf("BuildSubscription: %v", err)
	}
	if got, want := draft.DescriptorRef(), "easynet:///r/example/ability/hub.session.attach@1.0.0"; got != want {
		t.Fatalf("descriptor_ref = %q, want %q", got, want)
	}
	args := runtimeEventDraftArgs(t, draft)
	if _, ok := args["stream"]; ok {
		t.Fatalf("session draft should not carry generic stream arg: %#v", args)
	}
	if got := args["since_seq"]; got != float64(42) {
		t.Fatalf("since_seq = %#v, want 42", got)
	}
}

func TestRuntimeEventSubscriptionProviderRejectsMismatchedResumeCursor(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	ability, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}
	provider, err := NewRuntimeAbilityEventSubscriptionProvider(ability)
	if err != nil {
		t.Fatalf("NewRuntimeAbilityEventSubscriptionProvider: %v", err)
	}

	_, err = provider.BuildSubscription(context.Background(), RuntimeEventSubscriptionRequest{
		Call:         runtimeEventTestCall(),
		Stream:       RuntimeEventStreamDevice,
		ResumeCursor: &RuntimeEventSubscriptionCursor{Stream: "invocation", Sequence: 42},
	})
	if err == nil {
		t.Fatal("BuildSubscription accepted cross-stream resume cursor")
	}

	_, err = provider.BuildSubscription(context.Background(), RuntimeEventSubscriptionRequest{
		Call:         runtimeEventTestCall(),
		Stream:       RuntimeEventStreamDevice,
		ResumeCursor: &RuntimeEventSubscriptionCursor{Stream: "device", Sequence: 42, Token: " device:42 "},
	})
	if err == nil {
		t.Fatal("BuildSubscription accepted non-canonical resume token")
	}
}

func TestRuntimeEventSubscriptionAbilityRejectsUnsupportedStream(t *testing.T) {
	if _, err := RuntimeEventSubscriptionAbility(RuntimeEventStreamKind("legacy")); err == nil {
		t.Fatal("RuntimeEventSubscriptionAbility accepted unsupported stream")
	}
}

func runtimeEventTestCall() RuntimeCallContext {
	return RuntimeCallContext{
		CallerURA:         "easynet:///r/example/agent/backend",
		CalleeURA:         "easynet:///r/example/hub",
		SubjectURA:        "easynet:///r/example/user/system",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		Metadata:          map[string]any{"request_id": "events-test"},
	}
}

func runtimeEventDraftArgs(t *testing.T, draft InvocationDraft) map[string]any {
	t.Helper()
	raw, err := json.Marshal(draft.JSONArgs())
	if err != nil {
		t.Fatalf("marshal JSON args: %v", err)
	}
	args := map[string]any{}
	if err := json.Unmarshal(raw, &args); err != nil {
		t.Fatalf("unmarshal JSON args: %v", err)
	}
	return args
}
