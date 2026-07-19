package easynet

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestRuntimeAdminRoutesGeneratedFromManifest(t *testing.T) {
	_, source, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("runtime.Caller failed")
	}
	manifestPath := filepath.Join(
		filepath.Dir(source),
		"..",
		"..",
		"provider_routes",
		"easynet-runtime-admin-routes.v1.json",
	)
	manifest, err := os.ReadFile(manifestPath)
	if err != nil {
		t.Fatalf("read runtime admin route manifest: %v", err)
	}
	digest := sha256.Sum256(manifest)
	if got, want := runtimeAdminRouteManifestSHA256, fmt.Sprintf("%x", digest[:]); got != want {
		t.Fatalf("runtime admin route manifest digest = %s, want %s", got, want)
	}
}

func TestRuntimeAdminReadinessComposesLifecycleAndHealth(t *testing.T) {
	daemon := &memoryDaemonTransport{
		startJSON:  readyDaemonStatus(),
		statusJSON: `{"handle_id":"daemon-1","state":"Running","mode":"hub","endpoints":{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/daemon.sock"},"diagnostics":["status-ok"]}`,
	}
	control, err := NewRuntimeHost(daemon)
	if err != nil {
		t.Fatalf("NewRuntimeHost: %v", err)
	}
	health, err := NewHealthClient(staticHealthTransport{
		health:      []byte(`{"api_ready":true,"invocation_ready":true,"directory_ready":true,"trust_ready":true,"runtime_ready":true,"diagnostics":["health-ok"]}`),
		diagnostics: []byte(`{"profile":"health","kind":"diagnostics_report","state":"Running","ready":true,"version":"0.91.30","abi_version":5,"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/daemon.sock","checks":[{"name":"runtime","ready":true,"message":null}],"diagnostics":[]}`),
	})
	if err != nil {
		t.Fatalf("NewHealthClient: %v", err)
	}
	admin, err := NewRuntimeHostAdminClient(control, health)
	if err != nil {
		t.Fatalf("NewRuntimeHostAdminClient: %v", err)
	}
	handle, err := admin.Start(context.Background(), testRuntimeHostStartRequest{
		payload: map[string]any{"mode": "test-runtime"},
	})
	if err != nil {
		t.Fatalf("Start: %v", err)
	}

	readiness, err := admin.Readiness(context.Background(), handle)
	if err != nil {
		t.Fatalf("Readiness: %v", err)
	}
	if !readiness.Ready || readiness.LifecycleState != RuntimeRunning {
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
	control, err := NewRuntimeHost(&memoryDaemonTransport{})
	if err != nil {
		t.Fatalf("NewRuntimeHost: %v", err)
	}
	admin, err := NewRuntimeHostAdminClient(control, nil)
	if err != nil {
		t.Fatalf("NewRuntimeHostAdminClient: %v", err)
	}
	if _, err := admin.Status(context.Background(), nil); err == nil {
		t.Fatal("Status accepted nil handle")
	}
	if _, err := NewRuntimeHostAdminClient(nil, nil); err == nil {
		t.Fatal("NewRuntimeHostAdminClient accepted nil control")
	}
}

func TestRuntimeAdminUsesRuntimeHostLifecycleInterface(t *testing.T) {
	lifecycle := &runtimeAdminLifecycleStub{
		transport: &memoryDaemonTransport{
			statusJSON: `{"handle_id":"runtime-1","state":"Running","mode":"hub","endpoints":{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/runtime.sock"}}`,
		},
	}
	admin, err := NewRuntimeHostAdminClient(lifecycle, nil)
	if err != nil {
		t.Fatalf("NewRuntimeHostAdminClient: %v", err)
	}

	endpoints, err := admin.Discover(context.Background(), RuntimeHostDiscoverOptions{})
	if err != nil {
		t.Fatalf("Discover: %v", err)
	}
	if !lifecycle.discovered || endpoints.InvocationEndpoint != "unix:///tmp/runtime.sock" {
		t.Fatalf("runtime lifecycle interface was not used: endpoints=%#v discovered=%v", endpoints, lifecycle.discovered)
	}
	handle, err := admin.Start(context.Background(), testRuntimeHostStartRequest{
		payload: map[string]any{"mode": "test-runtime"},
	})
	if err != nil {
		t.Fatalf("Start: %v", err)
	}
	status, err := admin.Status(context.Background(), handle)
	if err != nil {
		t.Fatalf("Status: %v", err)
	}
	if !lifecycle.started || status.State != RuntimeRunning {
		t.Fatalf("runtime lifecycle start/status mismatch: state=%s started=%v", status.State, lifecycle.started)
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
			"hub_ura": "easynet:///r/example/authority",
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
	if got, want := capture.draft["descriptor_ref"], "easynet:///r/example/ability/authority.session.list@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"; got != want {
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

func TestRuntimeAdminAbilityClientAcceptsEmptySessions(t *testing.T) {
	capture := &runtimeAdminInvokeCapture{outputJSON: `{"state": "ok", "sessions": []}`}
	client := newRuntimeAdminAbilityTestClient(t, capture)

	page, err := client.ListSessions(context.Background(), RuntimeSessionListRequest{
		Call: runtimeAdminTestCall(),
	})
	if err != nil {
		t.Fatalf("ListSessions: %v", err)
	}
	if len(page.Sessions) != 0 {
		t.Fatalf("unexpected sessions: %#v", page.Sessions)
	}
}

func TestRuntimeAdminAbilityClientRejectsLegacySessionItems(t *testing.T) {
	capture := &runtimeAdminInvokeCapture{outputJSON: `{"state": "ok", "items": []}`}
	client := newRuntimeAdminAbilityTestClient(t, capture)

	_, err := client.ListSessions(context.Background(), RuntimeSessionListRequest{
		Call: runtimeAdminTestCall(),
	})
	if err == nil {
		t.Fatal("ListSessions accepted legacy items fallback")
	}
	if !strings.Contains(err.Error(), "sessions must be an array") {
		t.Fatalf("error = %v, want sessions array requirement", err)
	}
}

func TestRuntimeAdminAbilityClientRejectsMalformedSessionRows(t *testing.T) {
	capture := &runtimeAdminInvokeCapture{outputJSON: `{"state": "ok", "sessions": ["bad-row"]}`}
	client := newRuntimeAdminAbilityTestClient(t, capture)

	_, err := client.ListSessions(context.Background(), RuntimeSessionListRequest{
		Call: runtimeAdminTestCall(),
	})
	if err == nil {
		t.Fatal("ListSessions ignored malformed session row")
	}
	if !strings.Contains(err.Error(), "sessions entries must be objects") {
		t.Fatalf("error = %v, want sessions entry object requirement", err)
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
	if got, want := capture.draft["descriptor_ref"], "easynet:///r/example/ability/authority.federation.revoke@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"; got != want {
		t.Fatalf("descriptor_ref = %#v, want %q", got, want)
	}
	args := capture.args(t)
	if args["agent_ura"] != "easynet:///r/example/device/laptop" || args["reason"] != "owner_removed_device" {
		t.Fatalf("revoke args missing: %#v", args)
	}
}

func TestRuntimeAdminAbilityClientRejectsMissingRevokeAck(t *testing.T) {
	capture := &runtimeAdminInvokeCapture{outputJSON: `{"runtime_not_ready": false}`}
	client := newRuntimeAdminAbilityTestClient(t, capture)

	_, err := client.RevokeDevice(context.Background(), RuntimeDeviceRevokeRequest{
		Call:      runtimeAdminTestCall(),
		DeviceURA: "easynet:///r/example/device/laptop",
		Reason:    "owner_removed_device",
	})
	if err == nil {
		t.Fatal("RevokeDevice fabricated success without ack")
	}
	if !strings.Contains(err.Error(), "ack must be a boolean") {
		t.Fatalf("error = %v, want ack boolean requirement", err)
	}
}

func TestRuntimeAdminAbilityClientRejectsMalformedRevokeFlags(t *testing.T) {
	capture := &runtimeAdminInvokeCapture{outputJSON: `{"ack": true, "runtime_not_ready": "false"}`}
	client := newRuntimeAdminAbilityTestClient(t, capture)

	_, err := client.RevokeDevice(context.Background(), RuntimeDeviceRevokeRequest{
		Call:      runtimeAdminTestCall(),
		DeviceURA: "easynet:///r/example/device/laptop",
		Reason:    "owner_removed_device",
	})
	if err == nil {
		t.Fatal("RevokeDevice accepted malformed readiness flag")
	}
	if !strings.Contains(err.Error(), "runtime_not_ready must be a boolean") {
		t.Fatalf("error = %v, want runtime_not_ready boolean requirement", err)
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
	var output any
	if err := json.Unmarshal([]byte(c.outputJSON), &output); err != nil {
		return nil, err
	}
	admission, terminal := canonicalRuntimeReceiptPairFixture("inv-admin-test", "Completed")
	return mustJSON(map[string]any{
		"ok":                  true,
		"tuple":               c.draft,
		"invocation_id":       "inv-admin-test",
		"terminal_state":      "Completed",
		"output_content_type": "application/json",
		"output_json":         output,
		"elapsed_ms":          1,
		"admission_receipt":   admission,
		"terminal_receipt":    terminal,
		"error":               nil,
	}), nil
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
	runtimeClient, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc:               capture.Invoke,
		ResolveDescriptorRefFunc: testResolveDescriptorRef(t),
	})
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
		CalleeURA:         "easynet:///r/example/authority",
		SubjectURA:        "easynet:///r/example/resource/device.laptop/invoke/backend.admin",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		Metadata:          map[string]any{"request_id": "admin-test"},
	}
}

type runtimeAdminLifecycleStub struct {
	transport  *memoryDaemonTransport
	discovered bool
	started    bool
}

func (s *runtimeAdminLifecycleStub) DiscoverRuntime(context.Context, RuntimeHostDiscoverRequest) (Endpoints, error) {
	s.discovered = true
	return Endpoints{
		ControlEndpoint:    "unix:///tmp/control.sock",
		InvocationEndpoint: "unix:///tmp/runtime.sock",
	}, nil
}

func (s *runtimeAdminLifecycleStub) StartRuntime(context.Context, RuntimeHostStartRequest) (*RuntimeHandle, error) {
	s.started = true
	return newRuntimeHandle(s.transport, RuntimeLifecycleStatus{
		HandleID: "runtime-1",
		State:    RuntimeRunning,
		Mode:     RuntimeMode("test-runtime"),
		Endpoints: Endpoints{
			ControlEndpoint:    "unix:///tmp/control.sock",
			InvocationEndpoint: "unix:///tmp/runtime.sock",
		},
	})
}

func (s *runtimeAdminLifecycleStub) AttachRuntime(context.Context, RuntimeHostAttachOptions) (*RuntimeHandle, error) {
	return s.StartRuntime(context.Background(), testRuntimeHostStartRequest{
		payload: map[string]any{"mode": "test-runtime"},
	})
}

func (s *runtimeAdminLifecycleStub) ConnectLocal(context.Context, ConnectOptions) (*RuntimeClient, error) {
	return nil, invalidRuntimeClient("runtime client is not configured")
}
