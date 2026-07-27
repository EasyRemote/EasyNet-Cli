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

func TestRuntimeLifecycleTransportFuncRequiresDetachFunction(t *testing.T) {
	transport := RuntimeLifecycleTransportFunc{}

	err := transport.Detach(context.Background(), "daemon-1")
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Detach error = %v, want %s", err, ErrInvalidArgument)
	}
	if !strings.Contains(err.Error(), "runtime host detach transport function is required") {
		t.Fatalf("Detach error = %v", err)
	}
}

func TestConnectLocalFailsClosedWhenDetachProviderMissing(t *testing.T) {
	transport := RuntimeLifecycleTransportFunc{
		DiscoverFunc: func(context.Context, []byte) ([]byte, error) {
			return []byte(`{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/runtime.sock"}`), nil
		},
		AttachFunc: func(context.Context, []byte) ([]byte, error) {
			return []byte(`{
				"handle_id":"daemon-1",
				"state":"Running",
				"mode":"authority",
				"endpoints":{"invocation_endpoint":"unix:///tmp/runtime.sock"}
			}`), nil
		},
		OpenRuntimeFunc: func(context.Context, string, []byte) (RuntimeTransport, []byte, error) {
			return RuntimeTransportFunc{}, nil, nil
		},
	}

	client, err := ConnectLocalRuntimeHost(context.Background(), transport, ConnectOptions{})
	if client != nil {
		t.Fatalf("client = %#v, want nil", client)
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("ConnectLocalRuntimeHost error = %v, want %s", err, ErrInvalidArgument)
	}
	if !strings.Contains(err.Error(), "runtime host detach transport function is required") {
		t.Fatalf("ConnectLocalRuntimeHost error = %v", err)
	}
}
