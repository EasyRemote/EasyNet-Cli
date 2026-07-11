package easynet

import (
	"context"
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
