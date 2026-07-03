package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryAdminTransport struct {
	agentListInvocation    string
	agentStartInvocation   string
	agentStopInvocation    string
	agentRefreshInvocation string
	sessionListInvocation  string
	gatewayStatus          string
	agentRecords           string
	lifecycleResult        string
	seen                   map[string]map[string]any
}

func (m *memoryAdminTransport) remember(name string, requestJSON []byte) {
	if m.seen == nil {
		m.seen = map[string]map[string]any{}
	}
	var decoded map[string]any
	_ = json.Unmarshal(requestJSON, &decoded)
	m.seen[name] = decoded
}

func (m *memoryAdminTransport) BuildAgentListInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_agent_list", requestJSON)
	return []byte(m.agentListInvocation), nil
}

func (m *memoryAdminTransport) BuildAgentStartInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_agent_start", requestJSON)
	return []byte(m.agentStartInvocation), nil
}

func (m *memoryAdminTransport) BuildAgentStopInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_agent_stop", requestJSON)
	return []byte(m.agentStopInvocation), nil
}

func (m *memoryAdminTransport) BuildAgentRefreshInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_agent_refresh", requestJSON)
	return []byte(m.agentRefreshInvocation), nil
}

func (m *memoryAdminTransport) BuildSessionListInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_session_list", requestJSON)
	return []byte(m.sessionListInvocation), nil
}

func (m *memoryAdminTransport) GatewayStatus(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("gateway_status", requestJSON)
	return []byte(m.gatewayStatus), nil
}

func (m *memoryAdminTransport) ListAgents(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("list_agents", requestJSON)
	return []byte(m.agentRecords), nil
}

func (m *memoryAdminTransport) AgentStart(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("agent_start", requestJSON)
	return []byte(m.lifecycleResult), nil
}

func (m *memoryAdminTransport) AgentStop(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("agent_stop", requestJSON)
	return []byte(m.lifecycleResult), nil
}

func (m *memoryAdminTransport) AgentRefresh(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("agent_refresh", requestJSON)
	return []byte(m.lifecycleResult), nil
}

func (m *memoryAdminTransport) ListDeviceSessions(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("list_device_sessions", requestJSON)
	return []byte(`{"profile":"admin_gateway","kind":"device_sessions","state":"ok","items":[],"next_cursor":null,"metadata":{"profile":"admin_gateway","source":"session.list"}}`), nil
}

func adminBaseForTest() AdminCarrierBase {
	return AdminCarrierBase{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		Metadata:          map[string]any{"request_id": "admin-agent-list-1"},
	}
}

func TestAdminBuildsAgentAndSessionInvocations(t *testing.T) {
	transport := &memoryAdminTransport{
		agentListInvocation:    adminAgentListInvocationJSON,
		agentStartInvocation:   adminAgentStartInvocationJSON,
		agentStopInvocation:    adminAgentStopInvocationJSON,
		agentRefreshInvocation: adminAgentRefreshInvocationJSON,
		sessionListInvocation:  adminSessionListInvocationJSON,
	}
	client, err := NewAdminClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	listDraft, err := client.BuildAgentListInvocation(context.Background(), AdminAgentListRequest{AdminCarrierBase: adminBaseForTest()})
	if err != nil {
		t.Fatal(err)
	}
	if listDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0" {
		t.Fatalf("agent.list descriptor = %q", listDraft.DescriptorRef())
	}
	if transport.seen["build_agent_list"]["caller_ura"] != "easynet:///r/example/agent/alice.sdk" {
		t.Fatalf("admin carrier caller not preserved: %#v", transport.seen["build_agent_list"])
	}

	startReq := AdminAgentStartRequest{
		AdminCarrierBase: adminBaseForTest(),
		Name:             "codex",
		AgentType:        "codex",
		Model:            "gpt-5",
		Label:            "primary",
	}
	startDraft, err := client.BuildAgentStartInvocation(context.Background(), startReq)
	if err != nil {
		t.Fatal(err)
	}
	if startDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0" {
		t.Fatalf("agent.start descriptor = %q", startDraft.DescriptorRef())
	}
	if got := transport.seen["build_agent_start"]["name"]; got != "codex" {
		t.Fatalf("agent.start name = %#v", got)
	}

	stopDraft, err := client.BuildAgentStopInvocation(context.Background(), AdminAgentStopRequest{AdminCarrierBase: adminBaseForTest(), Name: "codex"})
	if err != nil {
		t.Fatal(err)
	}
	if stopDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0" {
		t.Fatalf("agent.stop descriptor = %q", stopDraft.DescriptorRef())
	}

	refreshDraft, err := client.BuildAgentRefreshInvocation(context.Background(), AdminAgentRefreshRequest{AdminCarrierBase: adminBaseForTest(), Name: "codex"})
	if err != nil {
		t.Fatal(err)
	}
	if refreshDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0" {
		t.Fatalf("agent.refresh descriptor = %q", refreshDraft.DescriptorRef())
	}

	includeTerminated := false
	sessionDraft, err := client.BuildSessionListInvocation(context.Background(), AdminSessionListRequest{AdminCarrierBase: adminBaseForTest(), IncludeTerminated: &includeTerminated})
	if err != nil {
		t.Fatal(err)
	}
	if sessionDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.session.list@1.0.0" {
		t.Fatalf("session.list descriptor = %q", sessionDraft.DescriptorRef())
	}
	if got := transport.seen["build_session_list"]["include_terminated"]; got != false {
		t.Fatalf("include_terminated = %#v", got)
	}
}

func TestAdminProjectsGatewayAgentsAndLifecycle(t *testing.T) {
	transport := &memoryAdminTransport{
		gatewayStatus:   adminGatewayStatusJSON,
		agentRecords:    adminAgentRecordsJSON,
		lifecycleResult: adminLifecycleResultJSON,
	}
	client, err := NewAdminClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	status, err := client.GatewayStatus(context.Background(), AdminGatewayStatusRequest{})
	if err != nil {
		t.Fatal(err)
	}
	if !status.ControlReady || !status.RuntimeReady || status.PublicListenerReady {
		t.Fatalf("unexpected gateway flags: %#v", status)
	}
	if len(status.Listeners) != 2 || status.Metadata["source"] != "daemon_lifecycle_status" {
		t.Fatalf("unexpected gateway projection: %#v", status)
	}

	page, err := client.ListAgents(context.Background(), AdminAgentListRequest{AdminCarrierBase: adminBaseForTest()})
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Items) != 1 || page.Items[0].Name != "codex" || page.Items[0].Runtime != "codex" {
		t.Fatalf("unexpected admin agent page: %#v", page)
	}

	result, err := client.AgentStart(context.Background(), AdminAgentStartRequest{
		AdminCarrierBase: adminBaseForTest(),
		Name:             "codex",
		AgentType:        "codex",
	})
	if err != nil {
		t.Fatal(err)
	}
	if result.Operation != "agent.start" || result.State != "ok" || result.AgentURA == nil {
		t.Fatalf("unexpected lifecycle result: %#v", result)
	}
}

func TestAdminRejectsIncompleteCarrierAndSystemLifecycle(t *testing.T) {
	client, err := NewAdminClient(&memoryAdminTransport{agentStartInvocation: adminAgentStartInvocationJSON})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := client.BuildAgentStartInvocation(context.Background(), AdminAgentStartRequest{Name: "codex", AgentType: "codex"}); err == nil {
		t.Fatal("expected incomplete carrier rejection")
	}
	if _, err := client.BuildAgentStartInvocation(context.Background(), AdminAgentStartRequest{AdminCarrierBase: adminBaseForTest(), Name: "device", AgentType: "codex"}); err == nil {
		t.Fatal("expected device system-agent rejection")
	}
	if _, err := client.BuildAgentStartInvocation(context.Background(), AdminAgentStartRequest{AdminCarrierBase: adminBaseForTest(), Name: "../codex", AgentType: "codex"}); err == nil {
		t.Fatal("expected path-like agent name rejection")
	}
	if _, err := client.BuildAgentStopInvocation(context.Background(), AdminAgentStopRequest{AdminCarrierBase: adminBaseForTest(), AgentURA: "easynet:///r/example/device/dev-a"}); err == nil {
		t.Fatal("expected non-agent URA rejection")
	}
}

const adminAgentListInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-agent-list-1",
    "profile": "admin_gateway",
    "system_ability": "agent.list",
    "carrier_owner": "daemon_sdk"
  }
}`

const adminAgentStartInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"name": "codex", "agent_type": "codex", "model": "gpt-5", "label": "primary"},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-agent-start-1",
    "profile": "admin_gateway",
    "system_ability": "agent.start",
    "carrier_owner": "daemon_sdk"
  }
}`

const adminAgentStopInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"name": "codex"},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-agent-stop-1",
    "profile": "admin_gateway",
    "system_ability": "agent.stop",
    "carrier_owner": "daemon_sdk"
  }
}`

const adminAgentRefreshInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"name": "codex"},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-agent-refresh-1",
    "profile": "admin_gateway",
    "system_ability": "agent.refresh",
    "carrier_owner": "daemon_sdk"
  }
}`

const adminSessionListInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.session.list@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"include_terminated": false},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-session-list-1",
    "profile": "admin_gateway",
    "system_ability": "session.list",
    "carrier_owner": "daemon_sdk"
  }
}`

const adminGatewayStatusJSON = `{
  "profile": "admin_gateway",
  "gateway_id": "device:example:dev-a",
  "ready": true,
  "state": "ready",
  "process_live": true,
  "control_ready": true,
  "runtime_ready": true,
  "directory_ready": true,
  "trust_ready": true,
  "public_listener_ready": false,
  "listeners": [
    {"kind": "control", "endpoint": "/tmp/easynet-control.sock", "ready": true, "public": false},
    {"kind": "invocation", "endpoint": "/tmp/easynet-daemon.sock", "ready": true, "public": false}
  ],
  "identity": {"mode": "device", "realm": "example", "node_id": "dev-a"},
  "metadata": {
    "profile": "admin_gateway",
    "source": "daemon_lifecycle_status",
    "lifecycle_state": "running",
    "requires_public_listener": false
  }
}`

const adminAgentRecordsJSON = `{
  "profile": "admin_gateway",
  "kind": "agent_records",
  "state": "ok",
  "items": [{
    "name": "codex",
    "agent_ura": "easynet:///r/example/agent/alice.codex",
    "owner_ura": "easynet:///r/example/user/alice",
    "device_ura": null,
    "state": "registered",
    "runtime": "codex",
    "model": "gpt-5",
    "label": "primary",
    "abilities": [],
    "metadata": {
      "profile": "admin_gateway",
      "source": "agent.list",
      "root_path": "/tmp/easynet/agents/codex",
      "root_exists": true,
      "timeout_secs": 600
    }
  }],
  "next_cursor": null,
  "metadata": {"profile": "admin_gateway", "source": "agent.list", "count": 1}
}`

const adminLifecycleResultJSON = `{
  "profile": "admin_gateway",
  "kind": "agent_lifecycle_result",
  "operation": "agent.start",
  "state": "ok",
  "agent_ura": "easynet:///r/example/agent/alice.codex",
  "ack": null,
  "runtime_not_ready": false,
  "runtime_catalog_not_ready": false,
  "metadata": {
    "profile": "admin_gateway",
    "source": "agent_lifecycle",
    "runtime_registered": 3,
    "runtime_failed": 0,
    "runtime_removed": 0,
    "raw_result": {
      "agent_ura": "easynet:///r/example/agent/alice.codex",
      "replaced_prior": false,
      "runtime_registered": 3,
      "runtime_failed": 0,
      "runtime_removed": 0,
      "runtime_not_ready": false,
      "runtime_catalog_not_ready": false
    }
  }
}`
