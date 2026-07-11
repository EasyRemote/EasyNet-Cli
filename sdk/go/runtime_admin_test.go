package easynet

import (
	"context"
	"testing"
)

func TestRuntimeAdminReadinessComposesLifecycleAndHealth(t *testing.T) {
	daemon := &memoryDaemonTransport{
		startJSON:  readyDaemonStatus(),
		statusJSON: `{"handle_id":"daemon-1","state":"Running","mode":"hub","endpoints":{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/daemon.sock"},"diagnostics":["status-ok"]}`,
	}
	control, err := NewDaemonControl(daemon)
	if err != nil {
		t.Fatalf("NewDaemonControl: %v", err)
	}
	health, err := NewHealthClient(staticHealthTransport{
		health:      []byte(`{"api_ready":true,"daemon_ready":true,"invocation_ready":true,"directory_ready":true,"trust_ready":true,"runtime_ready":true,"diagnostics":["health-ok"]}`),
		diagnostics: []byte(`{"profile":"health","kind":"diagnostics_report","state":"Running","ready":true,"version":"0.91.30","abi_version":5,"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/daemon.sock","checks":[{"name":"runtime","ready":true,"message":null}],"diagnostics":[]}`),
	})
	if err != nil {
		t.Fatalf("NewHealthClient: %v", err)
	}
	admin, err := NewRuntimeAdminClient(control, health)
	if err != nil {
		t.Fatalf("NewRuntimeAdminClient: %v", err)
	}
	handle, err := admin.Start(context.Background(), StartConfig{Mode: ModeHub})
	if err != nil {
		t.Fatalf("Start: %v", err)
	}

	readiness, err := admin.Readiness(context.Background(), handle)
	if err != nil {
		t.Fatalf("Readiness: %v", err)
	}
	if !readiness.Ready || readiness.LifecycleState != DaemonRunning {
		t.Fatalf("unexpected readiness: %#v", readiness)
	}
	if len(readiness.Messages) != 2 {
		t.Fatalf("messages not merged: %#v", readiness.Messages)
	}
	if readiness.Diagnostics == nil || readiness.Diagnostics.Kind != "diagnostics_report" {
		t.Fatalf("diagnostics missing: %#v", readiness.Diagnostics)
	}
}

func TestRuntimeAdminRejectsMissingHandle(t *testing.T) {
	control, err := NewDaemonControl(&memoryDaemonTransport{})
	if err != nil {
		t.Fatalf("NewDaemonControl: %v", err)
	}
	admin, err := NewRuntimeAdminClient(control, nil)
	if err != nil {
		t.Fatalf("NewRuntimeAdminClient: %v", err)
	}
	if _, err := admin.Status(context.Background(), nil); err == nil {
		t.Fatal("Status accepted nil handle")
	}
	if _, err := NewRuntimeAdminClient(nil, nil); err == nil {
		t.Fatal("NewRuntimeAdminClient accepted nil control")
	}
}
