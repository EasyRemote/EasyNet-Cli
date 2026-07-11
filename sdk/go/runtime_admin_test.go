package easynet

import (
	"context"
	"encoding/json"
	"fmt"
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

func TestRuntimeAdminAbilityClientListsSessions(t *testing.T) {
	includeTerminated := false
	capture := &runtimeAdminInvokeCapture{outputJSON: `{
		"state": "ok",
		"sessions": [{
			"kind": "terminal",
			"session_id": "session-a",
			"device_ura": "easynet:///r/example/device/laptop",
			"hub_ura": "easynet:///r/example/hub",
			"state": "active",
			"session_kind": "pty",
			"created_unix_ms": 1714492800000,
			"expires_unix_ms": 1714496400000,
			"metadata": {"source": "daemon"}
		}]
	}`}
	client := newRuntimeAdminAbilityTestClient(t, capture)

	page, err := client.ListSessions(context.Background(), RuntimeSessionListRequest{
		Call:              runtimeAdminTestCall(),
		IncludeTerminated: &includeTerminated,
	})
	if err != nil {
		t.Fatalf("ListSessions: %v", err)
	}
	if len(page.Sessions) != 1 || page.Sessions[0].SessionID != "session-a" {
		t.Fatalf("unexpected session page: %#v", page)
	}
	if got, want := capture.draft["descriptor_ref"], "easynet:///r/example/ability/hub.session.list@1.0.0"; got != want {
		t.Fatalf("descriptor_ref = %#v, want %q", got, want)
	}
	args := capture.args(t)
	if args["include_terminated"] != false {
		t.Fatalf("include_terminated arg missing: %#v", args)
	}
	metadata := capture.metadata(t)
	if metadata["sdk_profile"] != runtimeAdminProfile || metadata["system_ability"] != runtimeAdminSessionListAbility {
		t.Fatalf("runtime admin metadata missing: %#v", metadata)
	}
}

func TestRuntimeAdminAbilityClientRevokesDevice(t *testing.T) {
	capture := &runtimeAdminInvokeCapture{outputJSON: `{"ack": false, "runtime_not_ready": true}`}
	client := newRuntimeAdminAbilityTestClient(t, capture)

	result, err := client.RevokeDevice(context.Background(), RuntimeDeviceRevokeRequest{
		Call:      runtimeAdminTestCall(),
		DeviceURA: "easynet:///r/example/device/laptop",
		Reason:    "owner_removed_device",
	})
	if err != nil {
		t.Fatalf("RevokeDevice: %v", err)
	}
	if result.Ack || !result.RuntimeNotReady || result.DeviceURA != "easynet:///r/example/device/laptop" {
		t.Fatalf("unexpected revoke result: %#v", result)
	}
	if got, want := capture.draft["descriptor_ref"], "easynet:///r/example/ability/hub.federation.revoke@1.0.0"; got != want {
		t.Fatalf("descriptor_ref = %#v, want %q", got, want)
	}
	args := capture.args(t)
	if args["agent_ura"] != "easynet:///r/example/device/laptop" || args["reason"] != "owner_removed_device" {
		t.Fatalf("revoke args missing: %#v", args)
	}
}

func TestRuntimeAdminAbilityClientRejectsInvalidRevokeBeforeInvoke(t *testing.T) {
	capture := &runtimeAdminInvokeCapture{outputJSON: `{"ack": true}`}
	client := newRuntimeAdminAbilityTestClient(t, capture)

	_, err := client.RevokeDevice(context.Background(), RuntimeDeviceRevokeRequest{
		Call:      runtimeAdminTestCall(),
		DeviceURA: "easynet:///r/example/device/laptop",
	})
	if err == nil {
		t.Fatal("RevokeDevice accepted missing reason")
	}
	if capture.called {
		t.Fatalf("runtime transport was called despite invalid request: %#v", capture.draft)
	}
}

type runtimeAdminInvokeCapture struct {
	called     bool
	draft      map[string]any
	outputJSON string
}

func (c *runtimeAdminInvokeCapture) Invoke(_ context.Context, draftJSON []byte) ([]byte, error) {
	c.called = true
	if err := json.Unmarshal(draftJSON, &c.draft); err != nil {
		return nil, err
	}
	return []byte(fmt.Sprintf(`{
		"ok": true,
		"tuple": %s,
		"invocation_id": "inv-admin-test",
		"terminal_state": "Completed",
		"output_content_type": "application/json",
		"output_json": %s,
		"elapsed_ms": 1,
		"error": null
	}`, draftJSON, c.outputJSON)), nil
}

func (c *runtimeAdminInvokeCapture) args(t *testing.T) map[string]any {
	t.Helper()
	args, ok := c.draft["args"].(map[string]any)
	if !ok {
		t.Fatalf("draft args missing: %#v", c.draft)
	}
	return args
}

func (c *runtimeAdminInvokeCapture) metadata(t *testing.T) map[string]any {
	t.Helper()
	metadata, ok := c.draft["metadata"].(map[string]any)
	if !ok {
		t.Fatalf("draft metadata missing: %#v", c.draft)
	}
	return metadata
}

func newRuntimeAdminAbilityTestClient(t *testing.T, capture *runtimeAdminInvokeCapture) *RuntimeAdminAbilityClient {
	t.Helper()
	runtimeClient, err := NewRuntimeClient(RuntimeTransportFunc{InvokeFunc: capture.Invoke})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	ability, err := NewRuntimeAbilityClient(runtimeClient, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}
	client, err := NewRuntimeAdminAbilityClient(ability)
	if err != nil {
		t.Fatalf("NewRuntimeAdminAbilityClient: %v", err)
	}
	return client
}

func runtimeAdminTestCall() RuntimeCallContext {
	return RuntimeCallContext{
		CallerURA:         "easynet:///r/example/agent/backend",
		CalleeURA:         "easynet:///r/example/hub",
		SubjectURA:        "easynet:///r/example/resource/device.laptop/invoke/backend.admin",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		Metadata:          map[string]any{"request_id": "admin-test"},
	}
}
