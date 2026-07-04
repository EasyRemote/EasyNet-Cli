package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryMissionTransport struct {
	runInvocationJSON     string
	runFileInvocationJSON string
	trackInvocationJSON   string
	cancelInvocationJSON  string
	statusJSON            string
	eventsJSON            string
	seenRequest           map[string]any
}

func (m *memoryMissionTransport) remember(requestJSON []byte) {
	_ = json.Unmarshal(requestJSON, &m.seenRequest)
}

func (m *memoryMissionTransport) BuildRunEALInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.runInvocationJSON), nil
}

func (m *memoryMissionTransport) BuildRunFileInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.runFileInvocationJSON), nil
}

func (m *memoryMissionTransport) BuildTrackInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.trackInvocationJSON), nil
}

func (m *memoryMissionTransport) BuildCancelInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.cancelInvocationJSON), nil
}

func (m *memoryMissionTransport) RunEAL(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.statusJSON), nil
}

func (m *memoryMissionTransport) RunFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.statusJSON), nil
}

func (m *memoryMissionTransport) Track(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.statusJSON), nil
}

func (m *memoryMissionTransport) Cancel(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.statusJSON), nil
}

func (m *memoryMissionTransport) Events(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.eventsJSON), nil
}

func newMemoryMissionTransport() *memoryMissionTransport {
	return &memoryMissionTransport{
		runInvocationJSON:     missionRunInvocationFixtureJSON,
		runFileInvocationJSON: missionRunInvocationFixtureJSON,
		trackInvocationJSON:   missionTrackInvocationFixtureJSON,
		cancelInvocationJSON:  missionCancelInvocationFixtureJSON,
		statusJSON:            missionStatusFixtureJSON,
		eventsJSON:            missionEventPageFixtureJSON,
	}
}

func baseMissionCarrier() MissionCarrierBase {
	return MissionCarrierBase{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		Metadata:          map[string]any{"request_id": "mission-run-1"},
	}
}

func TestMissionBuildsRunTrackCancelInvocations(t *testing.T) {
	transport := newMemoryMissionTransport()
	client, err := NewMissionClient(transport)
	if err != nil {
		t.Fatalf("NewMissionClient: %v", err)
	}

	run, err := client.BuildRunEALInvocation(context.Background(), MissionRunRequest{
		MissionCarrierBase: baseMissionCarrier(),
		Source:             "mission weather\nlet r = local.observe_health()",
		Label:              "weather",
	})
	if err != nil {
		t.Fatalf("BuildRunEALInvocation: %v", err)
	}
	if run.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.mission.run@1.0.0" || !run.HasJSONArgs() {
		t.Fatalf("unexpected run invocation: %#v", run)
	}

	trackReq := MissionTrackRequest{MissionCarrierBase: baseMissionCarrier(), MissionID: "2026-07-04_010203_weather"}
	track, err := client.BuildTrackInvocation(context.Background(), trackReq)
	if err != nil {
		t.Fatalf("BuildTrackInvocation: %v", err)
	}
	if track.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.mission.track@1.0.0" {
		t.Fatalf("unexpected track invocation: %#v", track)
	}
	cancel, err := client.BuildCancelInvocation(context.Background(), MissionCancelRequest(trackReq))
	if err != nil {
		t.Fatalf("BuildCancelInvocation: %v", err)
	}
	if cancel.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.mission.cancel@1.0.0" {
		t.Fatalf("unexpected cancel invocation: %#v", cancel)
	}
}

func TestMissionRunFileAndStatusProjection(t *testing.T) {
	transport := newMemoryMissionTransport()
	client, err := NewMissionClient(transport)
	if err != nil {
		t.Fatalf("NewMissionClient: %v", err)
	}

	_, err = client.BuildRunFileInvocation(context.Background(), MissionRunFileRequest{
		MissionCarrierBase: baseMissionCarrier(),
		Path:               "/tmp/easynet-sdk-demo.eal",
		Label:              "file-weather",
	})
	if err != nil {
		t.Fatalf("BuildRunFileInvocation: %v", err)
	}
	if transport.seenRequest["path"] != "/tmp/easynet-sdk-demo.eal" {
		t.Fatalf("file path not forwarded: %#v", transport.seenRequest)
	}

	status, err := client.Track(context.Background(), MissionTrackRequest{
		MissionCarrierBase: baseMissionCarrier(),
		MissionID:          "2026-07-04_010203_weather",
	})
	if err != nil {
		t.Fatalf("Track: %v", err)
	}
	if !status.Terminal || status.State != "partial" || len(status.ChildReceipts) != 1 || status.ChildReceipts[0].ReceiptURA == "" {
		t.Fatalf("unexpected status: %#v", status)
	}
	if status.ParentReceiptURA == nil || *status.ParentReceiptURA == "" {
		t.Fatalf("parent receipt not preserved: %#v", status)
	}
}

func TestMissionEventsProjection(t *testing.T) {
	transport := newMemoryMissionTransport()
	client, err := NewMissionClient(transport)
	if err != nil {
		t.Fatalf("NewMissionClient: %v", err)
	}

	page, err := client.Events(context.Background(), MissionEventListRequest{
		MissionCarrierBase: baseMissionCarrier(),
		MissionID:          "2026-07-04_010203_weather",
		CursorSequence:     4,
		Limit:              100,
	})
	if err != nil {
		t.Fatalf("Events: %v", err)
	}

	if page.Kind != "mission_event_page" || page.NextCursorSequence != 7 || len(page.Events) != 2 {
		t.Fatalf("unexpected event page: %#v", page)
	}
	if page.Events[0].Sequence != 4 || page.Events[0].EventType != "progress" || page.Events[1].Terminal != true {
		t.Fatalf("unexpected events: %#v", page.Events)
	}
	if transport.seenRequest["mission_id"] != "2026-07-04_010203_weather" || transport.seenRequest["cursor_sequence"] != float64(4) {
		t.Fatalf("events request not forwarded: %#v", transport.seenRequest)
	}
}

func TestMissionRejectsIncompleteCarrierAndPathLikeMissionID(t *testing.T) {
	client, err := NewMissionClient(newMemoryMissionTransport())
	if err != nil {
		t.Fatalf("NewMissionClient: %v", err)
	}
	base := baseMissionCarrier()
	base.SubjectURA = ""
	if _, err := client.BuildRunEALInvocation(context.Background(), MissionRunRequest{MissionCarrierBase: base, Source: "mission x"}); err == nil {
		t.Fatalf("incomplete mission carrier accepted")
	}
	if _, err := client.BuildTrackInvocation(context.Background(), MissionTrackRequest{MissionCarrierBase: baseMissionCarrier(), MissionID: "/tmp/run"}); err == nil {
		t.Fatalf("path-like mission id accepted")
	}
	if _, err := client.Events(context.Background(), MissionEventListRequest{MissionCarrierBase: baseMissionCarrier(), MissionID: "2026-07-04_010203_weather", CursorSequence: -1}); err == nil {
		t.Fatalf("negative mission event cursor accepted")
	}
}

const missionRunInvocationFixtureJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.mission.run@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"source": "mission weather\nlet r = local.observe_health()", "label": "weather"},
  "content_type": "application/json",
  "metadata": {
    "request_id": "mission-run-1",
    "profile": "mission",
    "system_ability": "mission.run",
    "carrier_owner": "daemon_sdk"
  }
}`

const missionTrackInvocationFixtureJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.mission.track@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"run_id": "2026-07-04_010203_weather"},
  "content_type": "application/json",
  "metadata": {"request_id": "mission-track-1", "profile": "mission", "system_ability": "mission.track", "carrier_owner": "daemon_sdk"}
}`

const missionCancelInvocationFixtureJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.mission.cancel@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"run_id": "2026-07-04_010203_weather"},
  "content_type": "application/json",
  "metadata": {"request_id": "mission-cancel-1", "profile": "mission", "system_ability": "mission.cancel", "carrier_owner": "daemon_sdk"}
}`

const missionStatusFixtureJSON = `{
  "profile": "mission",
  "kind": "mission_status",
  "mission_id": "2026-07-04_010203_weather",
  "state": "partial",
  "terminal": true,
  "partial_failures": 1,
  "cancelled": false,
  "parent_invocation_id": null,
  "parent_receipt_ura": "easynet:///r/example/receipt/parent",
  "parent_invocation": {"caller": "easynet:///r/example/agent/alice.sdk"},
  "child_invocations": [
    {
      "step_id": "s1",
      "request_id": "req-1",
      "trace_id": "2026-07-04_010203_weather",
      "ability": "observe.health",
      "invocation_ura": "easynet:///r/example/invocation/req-1",
      "caller_ura": "easynet:///r/example/device/dev-a",
      "callee_ura": "easynet:///r/example/device/dev-a",
      "subject_ura": "easynet:///r/example/device/dev-a",
      "metadata_state": "receipt_backed",
      "ledger_state": "completed",
      "receipt": {"receipt_ura": "easynet:///r/example/receipt/child", "receipt_hash": "bbbb", "head_receipt_hash": "bbbb"}
    }
  ],
  "child_receipts": [{"step_id": "s1", "invocation_ura": "easynet:///r/example/invocation/req-1", "receipt_ura": "easynet:///r/example/receipt/child", "receipt_hash": "bbbb"}],
  "output_refs": [{"kind": "run_dir", "path": "/tmp/easynet/missions/runs/2026-07-04_010203_weather"}],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}`

const missionEventPageFixtureJSON = `{
  "profile": "mission",
  "kind": "mission_event_page",
  "mission_id": "2026-07-04_010203_weather",
  "cursor_sequence": 4,
  "next_cursor_sequence": 7,
  "has_more": false,
  "dropped_count": 0,
  "events": [
    {
      "profile": "mission",
      "kind": "mission_event",
      "mission_id": "2026-07-04_010203_weather",
      "sequence": 4,
      "event_type": "progress",
      "occurred_unix_ms": 1004,
      "terminal": false,
      "payload": {"delta": "hello"},
      "receipt": {},
      "metadata": {"step_id": "s1"}
    },
    {
      "profile": "mission",
      "kind": "mission_event",
      "mission_id": "2026-07-04_010203_weather",
      "sequence": 6,
      "event_type": "completed",
      "occurred_unix_ms": 1006,
      "terminal": true,
      "payload": {"reply": "done"},
      "receipt": {"receipt_ura": "easynet:///r/example/receipt/terminal"},
      "metadata": {}
    }
  ],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}`
