package easynet

import "testing"

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
