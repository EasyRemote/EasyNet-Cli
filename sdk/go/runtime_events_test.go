package easynet

import (
	"context"
	"testing"
)

func TestRuntimeEventClientReadsBoundedTypedPage(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		HandleEventsFunc: func(context.Context, InvocationControlCapability) ([]byte, error) {
			return []byte(`{"handle_id":7,"state":"Completed","terminal":true,"events":[{"sequence":1,"kind":"submitted","state":"Submitted","terminal":false},{"sequence":2,"kind":"completed","state":"Completed","terminal":true}],"result":{"ok":true}}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	provider, err := NewRuntimeHandleEventProvider(runtime)
	if err != nil {
		t.Fatalf("NewRuntimeHandleEventProvider: %v", err)
	}
	handle, err := newRuntimeInvocationHandleFromJSON([]byte(`{"handle_id":7,"state":"Submitted","terminal":false,"events":[],"result":null}`))
	if err != nil {
		t.Fatalf("newRuntimeInvocationHandleFromJSON: %v", err)
	}
	page, err := provider.ReadEvents(context.Background(), RuntimeEventReadRequest{Handle: handle, Cursor: &RuntimeEventCursor{Sequence: 1}, Limit: 1})
	if err != nil {
		t.Fatalf("ReadEvents: %v", err)
	}
	if len(page.Events) != 1 || page.Events[0].Sequence != 2 || !page.Terminal {
		t.Fatalf("unexpected page: %#v", page)
	}
}

func TestRuntimeEventClientTreatsFailedInvocationAsTerminalFeed(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		HandleEventsFunc: func(context.Context, InvocationControlCapability) ([]byte, error) {
			return []byte(`{"handle_id":7,"state":"Failed","terminal":true,"events":[{"sequence":1,"kind":"submitted","state":"Submitted","terminal":false},{"sequence":2,"kind":"failed","state":"Failed","terminal":true}],"result":{"ok":false}}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	provider, err := NewRuntimeHandleEventProvider(runtime)
	if err != nil {
		t.Fatalf("NewRuntimeHandleEventProvider: %v", err)
	}
	handle, err := newRuntimeInvocationHandleFromJSON([]byte(`{"handle_id":7,"state":"Submitted","terminal":false,"events":[],"result":null}`))
	if err != nil {
		t.Fatalf("newRuntimeInvocationHandleFromJSON: %v", err)
	}
	page, err := provider.ReadEvents(context.Background(), RuntimeEventReadRequest{Handle: handle})
	if err != nil {
		t.Fatalf("ReadEvents: %v", err)
	}
	if page.State != RuntimeEventStreamTerminal || !page.Terminal {
		t.Fatalf("failed invocation must close a healthy event feed as terminal: %#v", page)
	}
	if got := page.Events[len(page.Events)-1].State; got != "Failed" {
		t.Fatalf("terminal invocation event state = %q, want Failed", got)
	}
}

func TestRuntimeEventClientRejectsUnboundedLimit(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		ResolveDescriptorRefFunc: testResolveDescriptorRef(t),
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	provider, err := NewRuntimeHandleEventProvider(runtime)
	if err != nil {
		t.Fatalf("NewRuntimeHandleEventProvider: %v", err)
	}
	handle, err := newRuntimeInvocationHandleFromJSON([]byte(`{"handle_id":7,"state":"Submitted","terminal":false,"events":[],"result":null}`))
	if err != nil {
		t.Fatalf("newRuntimeInvocationHandleFromJSON: %v", err)
	}
	if _, err := provider.ReadEvents(context.Background(), RuntimeEventReadRequest{Handle: handle, Limit: MaxRuntimeEventPageLimit + 1}); err == nil {
		t.Fatal("ReadEvents accepted unbounded limit")
	}
}
