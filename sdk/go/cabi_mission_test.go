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

func TestCABIMissionTransportBuildsRunsAndProjects(t *testing.T) {
	libraryPath := buildFakeCABIMissionLibrary(t)
	client, transport, err := NewCABIMissionClient(libraryPath, "/tmp/easynet-control.json")
	if err != nil {
		t.Fatalf("NewCABIMissionClient: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("Close C ABI mission transport: %v", err)
		}
	}()

	runDraft, err := client.BuildRunEALInvocation(context.Background(), MissionRunRequest{
		MissionCarrierBase: baseMissionCarrier(),
		Source:             "mission weather\nlet r = local.observe_health()",
		Label:              "weather",
	})
	if err != nil {
		t.Fatalf("BuildRunEALInvocation: %v", err)
	}
	if runDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.mission.run@1.0.0" {
		t.Fatalf("run descriptor_ref = %q", runDraft.DescriptorRef())
	}

	run, err := client.RunEAL(context.Background(), MissionRunRequest{
		MissionCarrierBase: baseMissionCarrier(),
		Source:             "mission weather\nlet r = local.observe_health()",
		Label:              "weather",
	})
	if err != nil {
		t.Fatalf("RunEAL: %v", err)
	}
	if run.Status.MissionID != "2026-07-04_010203_weather" || run.Status.State != "completed" || !run.Status.Terminal {
		t.Fatalf("run status = %#v", run.Status)
	}

	statusRaw, err := transport.ProjectStatus(context.Background(), []byte(fakeCABIMissionStatusJSON))
	if err != nil {
		t.Fatalf("ProjectStatus: %v", err)
	}
	status, err := NewMissionStatusFromJSON(statusRaw)
	if err != nil {
		t.Fatalf("NewMissionStatusFromJSON: %v", err)
	}
	if status.MissionID != "2026-07-04_010203_weather" {
		t.Fatalf("projected status = %#v", status)
	}

	eventsRaw, err := transport.ProjectEvents(context.Background(), []byte(fakeCABIMissionEventsJSON))
	if err != nil {
		t.Fatalf("ProjectEvents: %v", err)
	}
	events, err := NewMissionEventPageFromJSON(eventsRaw)
	if err != nil {
		t.Fatalf("NewMissionEventPageFromJSON: %v", err)
	}
	if len(events.Events) != 2 || events.NextCursorSequence != 7 {
		t.Fatalf("projected events = %#v", events)
	}

	page, err := client.Events(context.Background(), MissionEventListRequest{
		MissionCarrierBase: baseMissionCarrier(),
		MissionID:          "2026-07-04_010203_weather",
		CursorSequence:     4,
		Limit:              25,
	})
	if err != nil {
		t.Fatalf("Events: %v", err)
	}
	if len(page.Events) != 2 || page.CursorSequence != 4 || page.NextCursorSequence != 7 {
		t.Fatalf("events page = %#v", page)
	}
}

func TestCABIMissionTransportClosed(t *testing.T) {
	libraryPath := buildFakeCABIMissionLibrary(t)
	client, transport, err := NewCABIMissionClient(libraryPath, "")
	if err != nil {
		t.Fatalf("NewCABIMissionClient: %v", err)
	}

	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if _, err := client.BuildTrackInvocation(context.Background(), MissionTrackRequest{
		MissionCarrierBase: baseMissionCarrier(),
		MissionID:          "2026-07-04_010203_weather",
	}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("BuildTrackInvocation after close error = %v, want %s", err, ErrInvalidArgument)
	}
}

func buildFakeCABIMissionLibrary(t *testing.T) string {
	t.Helper()
	cc, err := exec.LookPath("cc")
	if err != nil {
		t.Skip("cc is required for C ABI dynamic-library test")
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "fake_easynet_cli_mission.c")
	output := filepath.Join(dir, "libeasynet_cli.so")
	args := []string{"-shared", "-fPIC", "-o", output, source}
	if runtime.GOOS == "darwin" {
		output = filepath.Join(dir, "libeasynet_cli.dylib")
		args = []string{"-dynamiclib", "-o", output, source}
	}
	if err := os.WriteFile(source, []byte(fakeCABIMissionSource), 0o600); err != nil {
		t.Fatalf("write fake C ABI mission source: %v", err)
	}
	cmd := exec.Command(cc, args...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("build fake C ABI mission library: %v\n%s", err, out)
	}
	return output
}

const fakeCABIMissionStatusJSON = `{
  "mission_id": "2026-07-04_010203_weather",
  "meta": {
    "trace_id": "2026-07-04_010203_weather",
    "status": "completed",
    "steps_failed": 0,
    "invocation_context": {"receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/parent/receipt"},
    "child_invocations": []
  },
  "output_refs": [{"kind": "run_dir", "path": "/tmp/easynet/missions/runs/2026-07-04_010203_weather"}]
}`

const fakeCABIMissionEventsJSON = `{
  "mission_id": "2026-07-04_010203_weather",
  "cursor_sequence": 4,
  "has_more": false,
  "dropped_count": 0,
  "events": [
    {"sequence": 6, "event_type": "completed", "occurred_unix_ms": 1783219200006, "terminal": true, "payload": {"ok": true}, "receipt": {"receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/child/receipt"}},
    {"sequence": 4, "event_type": "progress", "occurred_unix_ms": 1783219200004, "terminal": false, "payload": {"step": "s1"}, "receipt": {}}
  ]
}`

const fakeCABIMissionSource = `
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
	*out_error_json = dup_json("{\"code\":\"GENERIC\",\"stage\":\"fake\",\"message\":\"fake C ABI mission error\",\"retry\":\"never\",\"details\":{}}");
	return 0;
}
int32_t easynet_init(const char *control_path, uint64_t *out_handle) {
	if (control_path != 0 && strstr(control_path, "control.json") == 0) return 10;
	*out_handle = 707;
	return 0;
}
int32_t easynet_shutdown(uint64_t handle) { (void)handle; return 0; }
int32_t easynet_invocation_invoke(uint64_t handle, const char *invocation_json, char **out_result_json) {
	(void)handle;
	if (strstr(invocation_json, "mission.events") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"has_more\":false,\"dropped_count\":0,\"events\":[{\"sequence\":4,\"event_type\":\"progress\",\"occurred_unix_ms\":1783219200004,\"terminal\":false,\"payload\":{\"step\":\"s1\"},\"receipt\":{}},{\"sequence\":6,\"event_type\":\"completed\",\"occurred_unix_ms\":1783219200006,\"terminal\":true,\"payload\":{\"ok\":true},\"receipt\":{\"receipt_ura\":\"easynet:///r/example/resource/agent.alice.sdk/invocation/child/receipt\"}}]},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "mission.run") == 0 && strstr(invocation_json, "mission.track") == 0 && strstr(invocation_json, "mission.cancel") == 0) return 10;
	*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"mission_id\":\"2026-07-04_010203_weather\",\"meta\":{\"trace_id\":\"2026-07-04_010203_weather\",\"status\":\"completed\",\"steps_failed\":0,\"invocation_context\":{\"receipt_ura\":\"easynet:///r/example/resource/agent.alice.sdk/invocation/parent/receipt\"},\"child_invocations\":[]},\"output_refs\":[{\"kind\":\"run_dir\",\"path\":\"/tmp/easynet/missions/runs/2026-07-04_010203_weather\"}]},\"error\":null}");
	return 0;
}
int32_t easynet_mission_build_run_eal_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "mission weather") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.mission.run@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"source\":\"mission weather\\nlet r = local.observe_health()\",\"label\":\"weather\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"mission\",\"system_ability\":\"mission.run\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_mission_build_run_file_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle; (void)request_json;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.mission.run@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"source\":\"mission file\",\"label\":\"file\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"mission\",\"system_ability\":\"mission.run\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_mission_build_track_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "2026-07-04_010203_weather") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.mission.track@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"run_id\":\"2026-07-04_010203_weather\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"mission\",\"system_ability\":\"mission.track\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_mission_build_cancel_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "2026-07-04_010203_weather") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.mission.cancel@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"run_id\":\"2026-07-04_010203_weather\"},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"mission\",\"system_ability\":\"mission.cancel\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_mission_build_events_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "2026-07-04_010203_weather") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.mission.events@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"run_id\":\"2026-07-04_010203_weather\",\"cursor_sequence\":4,\"limit\":25},\"content_type\":\"application/json\",\"metadata\":{\"profile\":\"mission\",\"system_ability\":\"mission.events\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_mission_project_status(uint64_t handle, const char *status_json, char **out_status_json) {
	(void)handle; (void)status_json;
	*out_status_json = dup_json("{\"profile\":\"mission\",\"kind\":\"mission_status\",\"mission_id\":\"2026-07-04_010203_weather\",\"state\":\"completed\",\"terminal\":true,\"partial_failures\":0,\"cancelled\":false,\"parent_invocation_id\":null,\"parent_receipt_ura\":\"easynet:///r/example/resource/agent.alice.sdk/invocation/parent/receipt\",\"parent_invocation\":{\"receipt_ura\":\"easynet:///r/example/resource/agent.alice.sdk/invocation/parent/receipt\"},\"child_invocations\":[],\"child_receipts\":[],\"output_refs\":[{\"kind\":\"run_dir\",\"path\":\"/tmp/easynet/missions/runs/2026-07-04_010203_weather\"}],\"error\":null,\"metadata\":{\"profile\":\"mission\",\"carrier_owner\":\"daemon_sdk\",\"status_source\":\"mission_result\"}}");
	return 0;
}
int32_t easynet_mission_project_events(uint64_t handle, const char *events_json, char **out_page_json) {
	(void)handle; (void)events_json;
	*out_page_json = dup_json("{\"profile\":\"mission\",\"kind\":\"mission_event_page\",\"mission_id\":\"2026-07-04_010203_weather\",\"cursor_sequence\":4,\"next_cursor_sequence\":7,\"has_more\":false,\"dropped_count\":0,\"events\":[{\"profile\":\"mission\",\"kind\":\"mission_event\",\"mission_id\":\"2026-07-04_010203_weather\",\"sequence\":4,\"event_type\":\"progress\",\"occurred_unix_ms\":1783219200004,\"terminal\":false,\"payload\":{\"step\":\"s1\"},\"receipt\":{},\"metadata\":{}},{\"profile\":\"mission\",\"kind\":\"mission_event\",\"mission_id\":\"2026-07-04_010203_weather\",\"sequence\":6,\"event_type\":\"completed\",\"occurred_unix_ms\":1783219200006,\"terminal\":true,\"payload\":{\"ok\":true},\"receipt\":{\"receipt_ura\":\"easynet:///r/example/resource/agent.alice.sdk/invocation/child/receipt\"},\"metadata\":{}}],\"metadata\":{\"profile\":\"mission\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
`
