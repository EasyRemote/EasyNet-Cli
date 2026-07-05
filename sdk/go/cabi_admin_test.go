//go:build easynet_cabi && cgo && !windows

package easynet

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"
)

func TestCABIAdminTransportBuildsInvokesAndProjects(t *testing.T) {
	libraryPath := buildFakeCABIAdminLibrary(t)
	client, transport, err := NewCABIAdminClient(libraryPath, "/tmp/easynet-control.json")
	if err != nil {
		t.Fatalf("NewCABIAdminClient: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("Close C ABI admin transport: %v", err)
		}
	}()

	startDraft, err := client.BuildAgentStartInvocation(context.Background(), AdminAgentStartRequest{
		AdminCarrierBase: adminBaseForTest(),
		Name:             "codex",
		AgentType:        "codex",
		Model:            "gpt-5",
		Label:            "primary",
	})
	if err != nil {
		t.Fatalf("BuildAgentStartInvocation: %v", err)
	}
	if startDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0" {
		t.Fatalf("agent.start descriptor_ref = %q", startDraft.DescriptorRef())
	}

	page, err := client.ListAgents(context.Background(), AdminAgentListRequest{AdminCarrierBase: adminBaseForTest()})
	if err != nil {
		t.Fatalf("ListAgents: %v", err)
	}
	if len(page.Items) != 1 || page.Items[0].Name != "codex" {
		t.Fatalf("agent page = %#v", page)
	}

	result, err := client.AgentStart(context.Background(), AdminAgentStartRequest{
		AdminCarrierBase: adminBaseForTest(),
		Name:             "codex",
		AgentType:        "codex",
	})
	if err != nil {
		t.Fatalf("AgentStart: %v", err)
	}
	if result.Operation != "agent.start" || result.AgentURA == nil {
		t.Fatalf("agent start result = %#v", result)
	}

	includeTerminated := false
	sessions, err := client.ListDeviceSessions(context.Background(), AdminSessionListRequest{
		AdminCarrierBase:  adminBaseForTest(),
		IncludeTerminated: &includeTerminated,
	})
	if err != nil {
		t.Fatalf("ListDeviceSessions: %v", err)
	}
	if len(sessions.Items) != 1 || sessions.Items[0].SessionID != "dev-session-1" {
		t.Fatalf("session page = %#v", sessions)
	}

	session, err := client.CreateDeviceSession(context.Background(), CreateDeviceSessionRequest{
		AdminCarrierBase: adminBaseForTest(),
		DeviceURA:        "easynet:///r/example/device/dev-a",
		HubURA:           "easynet:///r/example/hub/main",
		SessionKind:      "remote_desktop",
		ExpiresUnixMS:    1893456000000,
	})
	if err != nil {
		t.Fatalf("CreateDeviceSession: %v", err)
	}
	if session.SessionID != "dev-session-1" || session.SessionKind != "remote_desktop" {
		t.Fatalf("created session = %#v", session)
	}

	deleted, err := client.DeleteDeviceSession(context.Background(), DeleteDeviceSessionRequest{
		AdminCarrierBase: adminBaseForTest(),
		SessionID:        "dev-session-1",
		Reason:           "operator closed",
	})
	if err != nil {
		t.Fatalf("DeleteDeviceSession: %v", err)
	}
	if deleted.Operation != adminAbilitySessionDelete || deleted.DeviceURA == nil ||
		*deleted.DeviceURA != "easynet:///r/example/device/dev-a" {
		t.Fatalf("delete session result = %#v", deleted)
	}

	revokeDraft, err := client.BuildRevokeDeviceInvocation(context.Background(), RevokeDeviceRequest{
		AdminCarrierBase: adminBaseForTest(),
		DeviceURA:        "easynet:///r/example/device/dev-a",
		Reason:           "operator/key rotation",
	})
	if err != nil {
		t.Fatalf("BuildRevokeDeviceInvocation: %v", err)
	}
	if revokeDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.federation.revoke@1.0.0" {
		t.Fatalf("revoke descriptor_ref = %q", revokeDraft.DescriptorRef())
	}

	revoked, err := client.RevokeDevice(context.Background(), RevokeDeviceRequest{
		AdminCarrierBase: adminBaseForTest(),
		DeviceURA:        "easynet:///r/example/device/dev-a",
		Reason:           "operator/key rotation",
	})
	if err != nil {
		t.Fatalf("RevokeDevice: %v", err)
	}
	if revoked.Operation != adminAbilityRevokeDevice || revoked.DeviceURA == nil ||
		*revoked.DeviceURA != "easynet:///r/example/device/dev-a" {
		t.Fatalf("revoke result = %#v", revoked)
	}
}

func TestCABIAdminTransportProjectsGatewayAndReportsUnsupported(t *testing.T) {
	libraryPath := buildFakeCABIAdminLibrary(t)
	client, transport, err := NewCABIAdminClient(libraryPath, "")
	if err != nil {
		t.Fatalf("NewCABIAdminClient: %v", err)
	}

	statusRaw, err := transport.ProjectGatewayStatus(context.Background(), []byte(`{"runtime_status":"running"}`))
	if err != nil {
		t.Fatalf("ProjectGatewayStatus: %v", err)
	}
	status, err := NewGatewayStatusFromJSON(statusRaw)
	if err != nil {
		t.Fatalf("NewGatewayStatusFromJSON: %v", err)
	}
	if !status.RuntimeReady || status.PublicListenerReady {
		t.Fatalf("gateway status = %#v", status)
	}

	gateway, err := client.GatewayStatus(context.Background(), AdminGatewayStatusRequest{})
	if err != nil {
		t.Fatalf("GatewayStatus: %v", err)
	}
	if !gateway.Ready || gateway.GatewayID != "device:example:dev-a" {
		t.Fatalf("gateway status = %#v", gateway)
	}
	if _, err := client.JoinHub(context.Background(), AdminJoinHubRequest{
		AdminCarrierBase: adminBaseForTest(),
		HubURA:           "easynet:///r/example/hub/main",
		DeviceURA:        "easynet:///r/example/device/dev-a",
	}); !IsCode(err, ErrNotImplemented) {
		t.Fatalf("JoinHub error = %v, want %s", err, ErrNotImplemented)
	}

	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if _, err := client.BuildAgentListInvocation(context.Background(), AdminAgentListRequest{AdminCarrierBase: adminBaseForTest()}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("BuildAgentListInvocation after close error = %v, want %s", err, ErrInvalidArgument)
	}
}

func buildFakeCABIAdminLibrary(t *testing.T) string {
	t.Helper()
	cc, err := exec.LookPath("cc")
	if err != nil {
		t.Skip("cc is required for C ABI dynamic-library test")
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "fake_easynet_cli_admin.c")
	output := filepath.Join(dir, "libeasynet_cli.so")
	args := []string{"-shared", "-fPIC", "-o", output, source}
	if runtime.GOOS == "darwin" {
		output = filepath.Join(dir, "libeasynet_cli.dylib")
		args = []string{"-dynamiclib", "-o", output, source}
	}
	if err := os.WriteFile(source, []byte(fakeCABIAdminSource), 0o600); err != nil {
		t.Fatalf("write fake C ABI admin source: %v", err)
	}
	cmd := exec.Command(cc, args...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("build fake C ABI admin library: %v\n%s", err, out)
	}
	return output
}

const fakeCABIAdminSource = `
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

static char *dup_json(const char *s) {
	size_t n = strlen(s);
	char *out = (char *)malloc(n + 1);
	if (out == 0) return 0;
	memcpy(out, s, n + 1);
	return out;
}

uint32_t easynet_abi_version(void) { return 4u; }
void easynet_string_free(char *s) { free(s); }
int32_t easynet_last_error_json(char **out_error_json) {
	*out_error_json = dup_json("{\"code\":\"GENERIC\",\"stage\":\"fake\",\"message\":\"fake C ABI admin error\",\"retry\":\"never\",\"details\":{}}");
	return 0;
}
int32_t easynet_init(const char *control_path, uint64_t *out_handle) {
	if (control_path != 0 && strstr(control_path, "control.json") == 0) return 10;
	*out_handle = 1001;
	return 0;
}
int32_t easynet_shutdown(uint64_t handle) { (void)handle; return 0; }
int32_t easynet_daemon_attach(const char *options_json, uint64_t *out_daemon_handle) {
	(void)options_json;
	*out_daemon_handle = 2002;
	return 0;
}
int32_t easynet_daemon_detach(uint64_t daemon_handle) { (void)daemon_handle; return 0; }
int32_t easynet_daemon_status(uint64_t daemon_handle, char **out_status_json) {
	(void)daemon_handle;
	*out_status_json = dup_json("{\"pid\":4242,\"pid_alive\":true,\"control_accepting\":true,\"invocation_accepting\":true,\"control_endpoint\":\"/tmp/easynet-control.sock\",\"invocation_endpoint\":\"/tmp/easynet-daemon.sock\"}");
	return 0;
}
int32_t easynet_invocation_invoke(uint64_t handle, const char *invocation_json, char **out_result_json) {
	(void)handle;
	if (strstr(invocation_json, "agent.start") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"agent_ura\":\"easynet:///r/example/agent/alice.codex\",\"replaced_prior\":false,\"runtime_registered\":3,\"runtime_failed\":0,\"runtime_removed\":0,\"runtime_not_ready\":false,\"runtime_catalog_not_ready\":false},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "agent.list") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"agents\":[{\"name\":\"codex\",\"ura\":\"easynet:///r/example/agent/alice.codex\",\"runtime\":\"codex\",\"root_exists\":true,\"model\":\"gpt-5\",\"label\":\"primary\"}]},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "session.list") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"sessions\":[{\"id\":\"dev-session-1\",\"tenant\":\"example\",\"node\":\"dev-a\",\"started_unix_ms\":1767225600000,\"kind\":\"remote_desktop\",\"state\":\"active\"}]},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "session.create") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"session_id\":\"dev-session-1\",\"state\":\"active\",\"created_unix_ms\":1767225600000},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "session.delete") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"ack\":true,\"device_ura\":\"easynet:///r/example/device/dev-a\"},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "federation.revoke") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"ack\":true,\"was_active\":true},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "agent.stop") != 0 || strstr(invocation_json, "agent.refresh") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"agent_ura\":\"easynet:///r/example/agent/alice.codex\",\"runtime_removed\":1,\"runtime_registered\":0,\"runtime_failed\":0},\"error\":null}");
		return 0;
	}
	return 10;
}
int32_t easynet_admin_build_agent_list_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle; (void)request_json;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.agent.list@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"admin_gateway\",\"system_ability\":\"agent.list\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_admin_build_agent_start_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "codex") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.agent.start@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"name\":\"codex\",\"agent_type\":\"codex\",\"model\":\"gpt-5\",\"label\":\"primary\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"admin_gateway\",\"system_ability\":\"agent.start\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_admin_build_agent_stop_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "codex") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"name\":\"codex\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"admin_gateway\",\"system_ability\":\"agent.stop\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_admin_build_agent_refresh_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle; (void)request_json;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"name\":\"codex\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"admin_gateway\",\"system_ability\":\"agent.refresh\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_admin_build_session_list_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle; (void)request_json;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.session.list@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"include_terminated\":false},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"admin_gateway\",\"system_ability\":\"session.list\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_admin_build_session_create_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "remote_desktop") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.session.create@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"device_ura\":\"easynet:///r/example/device/dev-a\",\"hub_ura\":\"easynet:///r/example/hub/main\",\"session_kind\":\"remote_desktop\",\"expires_unix_ms\":1893456000000},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"admin_gateway\",\"system_ability\":\"session.create\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_admin_build_session_delete_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "dev-session-1") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.session.delete@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"session_id\":\"dev-session-1\",\"reason\":\"operator closed\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"admin_gateway\",\"system_ability\":\"session.delete\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_admin_build_revoke_device_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "operator/key rotation") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.federation.revoke@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"agent_ura\":\"easynet:///r/example/device/dev-a\",\"reason\":\"operator/key rotation\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"admin_gateway\",\"system_ability\":\"federation.revoke\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_admin_project_gateway_status(uint64_t handle, const char *status_json, char **out_status_json) {
	(void)handle; (void)status_json;
	*out_status_json = dup_json("{\"profile\":\"admin_gateway\",\"gateway_id\":\"device:example:dev-a\",\"ready\":true,\"state\":\"ready\",\"process_live\":true,\"control_ready\":true,\"runtime_ready\":true,\"directory_ready\":true,\"trust_ready\":true,\"public_listener_ready\":false,\"listeners\":[{\"kind\":\"control\",\"endpoint\":\"/tmp/easynet-control.sock\",\"ready\":true,\"public\":false},{\"kind\":\"invocation\",\"endpoint\":\"/tmp/easynet-daemon.sock\",\"ready\":true,\"public\":false}],\"identity\":{\"mode\":\"device\",\"realm\":\"example\",\"node_id\":\"dev-a\"},\"metadata\":{\"profile\":\"admin_gateway\",\"source\":\"daemon_lifecycle_status\",\"lifecycle_state\":\"running\",\"requires_public_listener\":false}}");
	return 0;
}
int32_t easynet_admin_project_agent_records(uint64_t handle, const char *agents_json, char **out_agents_json) {
	(void)handle; (void)agents_json;
	*out_agents_json = dup_json("{\"profile\":\"admin_gateway\",\"kind\":\"agent_records\",\"state\":\"ok\",\"items\":[{\"name\":\"codex\",\"agent_ura\":\"easynet:///r/example/agent/alice.codex\",\"owner_ura\":\"easynet:///r/example/user/alice\",\"device_ura\":null,\"state\":\"registered\",\"runtime\":\"codex\",\"model\":\"gpt-5\",\"label\":\"primary\",\"abilities\":[],\"metadata\":{\"profile\":\"admin_gateway\",\"source\":\"agent.list\",\"root_path\":\"/tmp/easynet/agents/codex\",\"root_exists\":true,\"timeout_secs\":600}}],\"next_cursor\":null,\"metadata\":{\"profile\":\"admin_gateway\",\"source\":\"agent.list\",\"count\":1}}");
	return 0;
}
int32_t easynet_admin_project_agent_lifecycle_result(uint64_t handle, const char *result_json, char **out_result_json) {
	(void)handle; (void)result_json;
	*out_result_json = dup_json("{\"profile\":\"admin_gateway\",\"kind\":\"agent_lifecycle_result\",\"operation\":\"agent.start\",\"state\":\"ok\",\"agent_ura\":\"easynet:///r/example/agent/alice.codex\",\"ack\":null,\"runtime_not_ready\":false,\"runtime_catalog_not_ready\":false,\"metadata\":{\"profile\":\"admin_gateway\",\"source\":\"agent_lifecycle\",\"runtime_registered\":3,\"runtime_failed\":0,\"runtime_removed\":0}}");
	return 0;
}
int32_t easynet_admin_project_device_session_page(uint64_t handle, const char *sessions_json, char **out_sessions_json) {
	(void)handle; (void)sessions_json;
	*out_sessions_json = dup_json("{\"profile\":\"admin_gateway\",\"kind\":\"device_sessions\",\"state\":\"ok\",\"items\":[{\"profile\":\"admin_gateway\",\"kind\":\"device_session\",\"session_id\":\"dev-session-1\",\"device_ura\":\"easynet:///r/example/device/dev-a\",\"hub_ura\":\"easynet:///r/example/hub/main\",\"state\":\"active\",\"session_kind\":\"remote_desktop\",\"created_unix_ms\":1767225600000,\"expires_unix_ms\":1893456000000,\"metadata\":{\"profile\":\"admin_gateway\",\"source\":\"session.list\"}}],\"next_cursor\":null,\"metadata\":{\"profile\":\"admin_gateway\",\"source\":\"session.list\"}}");
	return 0;
}
int32_t easynet_admin_project_device_session_result(uint64_t handle, const char *session_json, char **out_session_json) {
	(void)handle;
	if (strstr(session_json, "remote_desktop") == 0) return 10;
	*out_session_json = dup_json("{\"profile\":\"admin_gateway\",\"kind\":\"device_session\",\"session_id\":\"dev-session-1\",\"device_ura\":\"easynet:///r/example/device/dev-a\",\"hub_ura\":\"easynet:///r/example/hub/main\",\"state\":\"active\",\"session_kind\":\"remote_desktop\",\"created_unix_ms\":1767225600000,\"expires_unix_ms\":1893456000000,\"metadata\":{\"profile\":\"admin_gateway\",\"source\":\"session.create\"}}");
	return 0;
}
int32_t easynet_admin_project_device_admin_result(uint64_t handle, const char *result_json, char **out_result_json) {
	(void)handle;
	if (strstr(result_json, "session.delete") != 0) {
		*out_result_json = dup_json("{\"profile\":\"admin_gateway\",\"kind\":\"device_admin_result\",\"operation\":\"session.delete\",\"state\":\"ok\",\"agent_ura\":null,\"device_ura\":\"easynet:///r/example/device/dev-a\",\"ack\":true,\"runtime_not_ready\":false,\"runtime_catalog_not_ready\":false,\"metadata\":{\"profile\":\"admin_gateway\",\"source\":\"session.delete\"}}");
		return 0;
	}
	if (strstr(result_json, "federation.revoke") == 0) return 10;
	*out_result_json = dup_json("{\"profile\":\"admin_gateway\",\"kind\":\"device_admin_result\",\"operation\":\"federation.revoke\",\"state\":\"ok\",\"agent_ura\":null,\"device_ura\":\"easynet:///r/example/device/dev-a\",\"ack\":true,\"runtime_not_ready\":false,\"runtime_catalog_not_ready\":false,\"metadata\":{\"profile\":\"admin_gateway\",\"source\":\"federation.revoke\"}}");
	return 0;
}
`
