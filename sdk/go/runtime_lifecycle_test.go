package easynet

import (
	"context"
	"strings"
	"testing"
)

func TestRuntimeLifecycleStatusAcceptsGenericMode(t *testing.T) {
	status, err := NewRuntimeLifecycleStatusFromJSON([]byte(`{
		"state":"Running",
		"mode":"authority",
		"endpoints":{"invocation_endpoint":"unix:///tmp/runtime.sock"}
	}`))
	if err != nil {
		t.Fatalf("NewRuntimeLifecycleStatusFromJSON: %v", err)
	}
	if status.Mode != "authority" {
		t.Fatalf("mode = %q, want authority", status.Mode)
	}
}

func TestRuntimeLifecycleStatusRejectsRetiredProductMode(t *testing.T) {
	_, err := NewRuntimeLifecycleStatusFromJSON([]byte(`{
		"state":"Running",
		"mode":"hub",
		"endpoints":{"invocation_endpoint":"unix:///tmp/runtime.sock"}
	}`))
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("retired product mode error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestRuntimeLifecycleStatusKeepsUnknownAsParseableObservation(t *testing.T) {
	status, err := NewRuntimeLifecycleStatusFromJSON([]byte(`{
		"handle_id":"daemon-1",
		"state":"Unknown",
		"mode":"authority"
	}`))
	if err != nil {
		t.Fatalf("NewRuntimeLifecycleStatusFromJSON: %v", err)
	}
	if status.State != RuntimeUnknown {
		t.Fatalf("state = %q, want %q", status.State, RuntimeUnknown)
	}
}

func TestRuntimeLifecycleRejectsUnknownWildcardTransition(t *testing.T) {
	transport := RuntimeLifecycleTransportFunc{
		StatusFunc: func(context.Context, string) ([]byte, error) {
			return []byte(`{
				"handle_id":"daemon-1",
				"state":"Running",
				"mode":"authority",
				"endpoints":{"invocation_endpoint":"unix:///tmp/runtime.sock"}
			}`), nil
		},
	}
	handle, err := newRuntimeHandle(transport, RuntimeLifecycleStatus{
		HandleID: "daemon-1",
		State:    RuntimeUnknown,
		Mode:     "authority",
	})
	if err != nil {
		t.Fatalf("newRuntimeHandle: %v", err)
	}

	_, err = handle.Status(context.Background())
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Unknown wildcard transition error = %v, want %s", err, ErrInvalidArgument)
	}
	if !strings.Contains(err.Error(), "runtime lifecycle cannot transition from Unknown to Running") {
		t.Fatalf("Unknown wildcard transition error = %v", err)
	}
	if handle.State() != RuntimeUnknown {
		t.Fatalf("state = %q, want %q", handle.State(), RuntimeUnknown)
	}
}
