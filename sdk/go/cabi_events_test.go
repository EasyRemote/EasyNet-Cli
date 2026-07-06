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

func TestCABIEventsTransportBuildsCarriersAndProjectsFrames(t *testing.T) {
	libraryPath := buildFakeCABIEventsLibrary(t)
	client, transport, err := NewCABIEventClient(libraryPath, "/tmp/easynet-control.json")
	if err != nil {
		t.Fatalf("NewCABIEventClient: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("Close C ABI events transport: %v", err)
		}
	}()

	cursor, err := NewEventCursor("directory", 7)
	if err != nil {
		t.Fatalf("NewEventCursor: %v", err)
	}
	draft, err := client.BuildDirectorySubscriptionInvocation(context.Background(), EventsDirectorySubscriptionRequest{
		EventsCarrierBase:   eventsBaseForTest(),
		Realm:               "example",
		AgentURA:            "easynet:///r/example/agent/alice.main",
		ResumeCursor:        &cursor,
		HeartbeatIntervalMS: 30000,
	})
	if err != nil {
		t.Fatalf("BuildDirectorySubscriptionInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2@1.0.0" {
		t.Fatalf("directory descriptor_ref = %q", draft.DescriptorRef())
	}

	sessionCursor, err := NewEventCursor("session", 4)
	if err != nil {
		t.Fatalf("NewEventCursor(session): %v", err)
	}
	sessionDraft, err := client.BuildSessionSubscriptionInvocation(context.Background(), EventsSessionSubscriptionRequest{
		EventsCarrierBase: eventsBaseForTest(),
		SessionID:         "run-1",
		ResumeCursor:      &sessionCursor,
	})
	if err != nil {
		t.Fatalf("BuildSessionSubscriptionInvocation: %v", err)
	}
	if sessionDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.session.attach@1.0.0" {
		t.Fatalf("session descriptor_ref = %q", sessionDraft.DescriptorRef())
	}

	deviceCursor, err := NewEventCursor("device", 2)
	if err != nil {
		t.Fatalf("NewEventCursor(device): %v", err)
	}
	deviceDraft, err := client.BuildDeviceSubscriptionInvocation(context.Background(), EventsDeviceSubscriptionRequest{
		EventsCarrierBase: eventsBaseForTest(),
		DeviceURA:         "easynet:///r/example/device/dev-a",
		ResumeCursor:      &deviceCursor,
	})
	if err != nil {
		t.Fatalf("BuildDeviceSubscriptionInvocation: %v", err)
	}
	if deviceDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.events.device.subscribe@1.0.0" {
		t.Fatalf("device descriptor_ref = %q", deviceDraft.DescriptorRef())
	}

	invocationDraft, err := client.BuildInvocationSubscriptionInvocation(context.Background(), EventsInvocationSubscriptionRequest{
		EventsCarrierBase: eventsBaseForTest(),
		InvocationID:      "inv-1",
	})
	if err != nil {
		t.Fatalf("BuildInvocationSubscriptionInvocation: %v", err)
	}
	if invocationDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.events.invocation.subscribe@1.0.0" {
		t.Fatalf("invocation descriptor_ref = %q", invocationDraft.DescriptorRef())
	}

	page, err := client.ListDeviceEvents(context.Background(), EventsDeviceEventListRequest{
		EventsCarrierBase: eventsBaseForTest(),
		DeviceURA:         "easynet:///r/example/device/dev-a",
	})
	if err != nil {
		t.Fatalf("ListDeviceEvents: %v", err)
	}
	if len(page.Items) != 1 || page.Items[0].Kind != "device.status_changed" || page.Stream != "device" {
		t.Fatalf("device event page = %#v", page)
	}

	event, err := client.ProjectDirectoryEvent(context.Background(), EventProjectionInput{
		Cursor: cursor,
		Event: map[string]any{
			"type":       "agent_revoked",
			"agent_ura":  "easynet:///r/example/agent/alice.main",
			"was_active": true,
			"reason":     "stream_closed",
			"unix_ms":    1783100000123,
		},
	})
	if err != nil {
		t.Fatalf("ProjectDirectoryEvent: %v", err)
	}
	if event.Kind != "directory.agent_revoked" || event.Cursor.Token != "directory:8" {
		t.Fatalf("directory event = %#v", event)
	}

	dropCursor, _ := NewEventCursor("directory", 9)
	drop, err := client.ProjectDropReport(context.Background(), EventDropReportInput{
		Cursor:         dropCursor,
		OccurredUnixMS: 1783100000124,
		DroppedCount:   3,
	})
	if err != nil {
		t.Fatalf("ProjectDropReport: %v", err)
	}
	if drop.DroppedCount != 3 || drop.Kind != "directory.drop_report" {
		t.Fatalf("drop report = %#v", drop)
	}

	terminalCursor, _ := NewEventCursor("directory", 10)
	terminal, err := client.ProjectTerminal(context.Background(), EventTerminalInput{
		Cursor:         terminalCursor,
		OccurredUnixMS: 1783100000125,
		Reason:         "client_closed",
	})
	if err != nil {
		t.Fatalf("ProjectTerminal: %v", err)
	}
	if !terminal.Terminal || terminal.Kind != "directory.terminal" {
		t.Fatalf("terminal = %#v", terminal)
	}
}

func TestCABIEventsTransportStreamsThroughRuntimeCoreAndCloses(t *testing.T) {
	libraryPath := buildFakeCABIEventsLibrary(t)
	client, transport, err := NewCABIEventClient(libraryPath, "")
	if err != nil {
		t.Fatalf("NewCABIEventClient: %v", err)
	}

	stream, err := client.SubscribeDevices(context.Background(), EventsDeviceSubscriptionRequest{
		EventsCarrierBase: eventsBaseForTest(),
		DeviceURA:         "easynet:///r/example/device/dev-a",
	})
	if err != nil {
		t.Fatalf("SubscribeDevices: %v", err)
	}
	if stream.Stream != "device" || stream.StreamID != "77" || stream.State == "" {
		t.Fatalf("device stream = %#v", stream)
	}
	frame, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("device stream Next: %v", err)
	}
	if frame.Stream != "device" || frame.Kind != "device.status_changed" || frame.Cursor.Token != "device:8" {
		t.Fatalf("device live frame = %#v", frame)
	}
	cancel, err := stream.Cancel(context.Background(), "test")
	if err != nil {
		t.Fatalf("device stream Cancel: %v", err)
	}
	if !cancel.Cancelled() || cancel.StreamID() != "77" || cancel.State() != StreamCancelled {
		t.Fatalf("device stream cancel = %#v", cancel)
	}

	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if _, err := client.ProjectTerminal(context.Background(), EventTerminalInput{
		Cursor:         EventCursor{Stream: "directory", Sequence: 10},
		OccurredUnixMS: 1783100000125,
	}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("ProjectTerminal after close error = %v, want %s", err, ErrInvalidArgument)
	}
}

func buildFakeCABIEventsLibrary(t *testing.T) string {
	t.Helper()
	cc, err := exec.LookPath("cc")
	if err != nil {
		t.Skip("cc is required for C ABI dynamic-library test")
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "fake_easynet_cli_events.c")
	output := filepath.Join(dir, "libeasynet_cli.so")
	args := []string{"-shared", "-fPIC", "-o", output, source}
	if runtime.GOOS == "darwin" {
		output = filepath.Join(dir, "libeasynet_cli.dylib")
		args = []string{"-dynamiclib", "-o", output, source}
	}
	if err := os.WriteFile(source, []byte(fakeCABIEventsSource), 0o600); err != nil {
		t.Fatalf("write fake C ABI events source: %v", err)
	}
	cmd := exec.Command(cc, args...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("build fake C ABI events library: %v\n%s", err, out)
	}
	return output
}

const fakeCABIEventsSource = `
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
	*out_error_json = dup_json("{\"code\":\"GENERIC\",\"stage\":\"fake\",\"message\":\"fake C ABI events error\",\"retry\":\"never\",\"details\":{}}");
	return 0;
}
int32_t easynet_init(const char *control_path, uint64_t *out_handle) {
	if (control_path != 0 && strstr(control_path, "control.json") == 0) return 10;
	*out_handle = 909;
	return 0;
}
int32_t easynet_shutdown(uint64_t handle) { (void)handle; return 0; }
int32_t easynet_invocation_invoke(uint64_t handle, const char *invocation_json, char **out_result_json) {
	(void)handle;
	if (strstr(invocation_json, "events.device.history") == 0) return 10;
	*out_result_json = dup_json("{\"output_json\":{\"events\":[{\"sequence\":8,\"device_ura\":\"easynet:///r/example/device/dev-a\",\"occurred_unix_ms\":1783100000123,\"kind\":\"device.status_changed\",\"payload\":{\"state\":\"online\"}}]}}");
	return 0;
}
typedef void (*stream_cb)(void *user_data, const char *chunk_json);
int32_t easynet_invocation_stream_open(uint64_t handle, const char *invocation_json, stream_cb on_chunk, void *user_data, uint64_t *out_stream_id) {
	(void)handle;
	if (strstr(invocation_json, "federation.subscribe_directory_v2") == 0 &&
	    strstr(invocation_json, "events.device.subscribe") == 0 &&
	    strstr(invocation_json, "session.attach") == 0 &&
	    strstr(invocation_json, "events.invocation.subscribe") == 0) return 10;
	*out_stream_id = 77;
	if (on_chunk != 0) {
		on_chunk(user_data, "{\"sequence\":8,\"event\":\"chunk\",\"payload_content_type\":\"application/json\",\"payload_json\":{\"sequence\":8,\"device_ura\":\"easynet:///r/example/device/dev-a\",\"occurred_unix_ms\":1783100000123,\"kind\":\"device.status_changed\",\"payload\":{\"state\":\"online\"}},\"terminal\":false}");
	}
	return 0;
}
int32_t easynet_invocation_stream_cancel(uint64_t handle, uint64_t stream_id) {
	(void)handle; (void)stream_id;
	return 0;
}
int32_t easynet_invocation_stream_close(uint64_t handle, uint64_t stream_id) {
	(void)handle; (void)stream_id;
	return 0;
}
int32_t easynet_events_build_directory_subscription_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "directory") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"stream\":\"directory\",\"daemon_ability\":\"federation.subscribe_directory_v2\",\"realm\":\"example\",\"agent_ura\":\"easynet:///r/example/agent/alice.main\",\"resume_cursor\":\"directory:7\",\"heartbeat_interval_ms\":30000},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"events\",\"system_ability\":\"federation.subscribe_directory_v2\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_events_build_device_subscription_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "device") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.events.device.subscribe@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"stream\":\"device\",\"device_ura\":\"easynet:///r/example/device/dev-a\",\"resume_cursor\":\"device:2\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"events\",\"system_ability\":\"events.device.subscribe\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_events_build_session_subscription_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "run-1") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.session.attach@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"stream\":\"session\",\"daemon_ability\":\"session.attach\",\"session_id\":\"run-1\",\"since_seq\":4},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"events\",\"system_ability\":\"session.attach\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_events_build_invocation_subscription_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "inv-1") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.events.invocation.subscribe@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"stream\":\"invocation\",\"invocation_id\":\"inv-1\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"events\",\"system_ability\":\"events.invocation.subscribe\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_events_build_device_event_history_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "device") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.events.device.history@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"stream\":\"device\",\"device_ura\":\"easynet:///r/example/device/dev-a\",\"limit\":50},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"events\",\"system_ability\":\"events.device.history\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_events_project_device_event_page(uint64_t handle, const char *page_json, char **out_page_json) {
	(void)handle;
	if (strstr(page_json, "events") == 0) return 10;
	*out_page_json = dup_json("{\"profile\":\"events\",\"stream\":\"device\",\"item_kind\":\"device_event\",\"items\":[{\"profile\":\"events\",\"stream\":\"device\",\"kind\":\"device.status_changed\",\"event_id\":\"evt-device-8\",\"cursor\":{\"stream\":\"device\",\"sequence\":8,\"token\":\"device:8\"},\"resume_token\":\"device:8\",\"occurred_unix_ms\":1783100000123,\"occurred_at\":\"2026-07-03T17:33:20.123Z\",\"subject_ref\":{\"kind\":\"ura\",\"ura\":\"easynet:///r/example/device/dev-a\",\"role\":\"device\"},\"tenant_ref\":{\"kind\":\"realm\",\"realm\":\"example\"},\"payload\":{\"state\":\"online\"},\"dropped_count\":0,\"reconnect_after_ms\":null,\"terminal\":false,\"metadata\":{\"profile\":\"events\",\"stream\":\"device\",\"source\":\"daemon_device_event\"}}],\"next_cursor\":null,\"has_more\":false,\"limit\":50,\"metadata\":{\"profile\":\"events\",\"source\":\"device_event_history\"}}");
	return 0;
}
int32_t easynet_events_project_directory_event(uint64_t handle, const char *event_json, char **out_event_json) {
	(void)handle; (void)event_json;
	*out_event_json = dup_json("{\"profile\":\"events\",\"stream\":\"directory\",\"kind\":\"directory.agent_revoked\",\"event_id\":\"evt-directory-8\",\"cursor\":{\"stream\":\"directory\",\"sequence\":8,\"token\":\"directory:8\"},\"resume_token\":\"directory:8\",\"occurred_unix_ms\":1783100000123,\"occurred_at\":\"2026-07-03T17:33:20.123Z\",\"subject_ref\":{\"kind\":\"ura\",\"ura\":\"easynet:///r/example/agent/alice.main\",\"role\":\"agent\"},\"tenant_ref\":{\"kind\":\"realm\",\"realm\":\"example\"},\"payload\":{\"type\":\"agent_revoked\",\"agent_ura\":\"easynet:///r/example/agent/alice.main\",\"was_active\":true,\"reason\":\"stream_closed\",\"unix_ms\":1783100000123},\"dropped_count\":0,\"reconnect_after_ms\":null,\"terminal\":false,\"metadata\":{\"profile\":\"events\",\"stream\":\"directory\",\"carrier_owner\":\"daemon_sdk\",\"source\":\"daemon_directory_event\",\"stream_ability\":\"federation.subscribe_directory_v2\",\"lifecycle\":\"delta\",\"daemon_event_type\":\"agent_revoked\"}}");
	return 0;
}
int32_t easynet_events_project_live_event(uint64_t handle, const char *event_json, char **out_event_json) {
	(void)handle; (void)event_json;
	*out_event_json = dup_json("{\"profile\":\"events\",\"stream\":\"device\",\"kind\":\"device.status_changed\",\"event_id\":\"evt-device-8\",\"cursor\":{\"stream\":\"device\",\"sequence\":8,\"token\":\"device:8\"},\"resume_token\":\"device:8\",\"occurred_unix_ms\":1783100000123,\"occurred_at\":\"2026-07-03T17:33:20.123Z\",\"subject_ref\":{\"kind\":\"ura\",\"ura\":\"easynet:///r/example/device/dev-a\",\"role\":\"device\"},\"tenant_ref\":{\"kind\":\"realm\",\"realm\":\"example\"},\"payload\":{\"state\":\"online\"},\"dropped_count\":0,\"reconnect_after_ms\":null,\"terminal\":false,\"metadata\":{\"profile\":\"events\",\"stream\":\"device\",\"carrier_owner\":\"daemon_sdk\",\"source\":\"daemon_device_event\",\"stream_ability\":\"events.device.subscribe\",\"lifecycle\":\"live\"}}");
	return 0;
}
int32_t easynet_events_project_drop_report(uint64_t handle, const char *drop_json, char **out_event_json) {
	(void)handle; (void)drop_json;
	*out_event_json = dup_json("{\"profile\":\"events\",\"stream\":\"directory\",\"kind\":\"directory.drop_report\",\"event_id\":\"evt-directory-drop-9\",\"cursor\":{\"stream\":\"directory\",\"sequence\":9,\"token\":\"directory:9\"},\"resume_token\":\"directory:9\",\"occurred_unix_ms\":1783100000124,\"occurred_at\":\"2026-07-03T17:33:20.124Z\",\"subject_ref\":null,\"tenant_ref\":null,\"payload\":{\"reason\":\"overflow\"},\"dropped_count\":3,\"reconnect_after_ms\":1000,\"terminal\":false,\"metadata\":{\"profile\":\"events\",\"stream\":\"directory\",\"carrier_owner\":\"daemon_sdk\",\"source\":\"daemon_directory_event\",\"lifecycle\":\"drop_report\"}}");
	return 0;
}
int32_t easynet_events_project_terminal(uint64_t handle, const char *terminal_json, char **out_event_json) {
	(void)handle; (void)terminal_json;
	*out_event_json = dup_json("{\"profile\":\"events\",\"stream\":\"directory\",\"kind\":\"directory.terminal\",\"event_id\":\"evt-directory-terminal-10\",\"cursor\":{\"stream\":\"directory\",\"sequence\":10,\"token\":\"directory:10\"},\"resume_token\":\"directory:10\",\"occurred_unix_ms\":1783100000125,\"occurred_at\":\"2026-07-03T17:33:20.125Z\",\"subject_ref\":null,\"tenant_ref\":null,\"payload\":{\"reason\":\"client_closed\"},\"dropped_count\":0,\"reconnect_after_ms\":null,\"terminal\":true,\"metadata\":{\"profile\":\"events\",\"stream\":\"directory\",\"carrier_owner\":\"daemon_sdk\",\"source\":\"daemon_directory_event\",\"lifecycle\":\"terminal\"}}");
	return 0;
}
`
