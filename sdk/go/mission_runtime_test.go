package easynet

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestMissionRuntimeTransportInvokesStatusThroughRuntimeCore(t *testing.T) {
	ctx := context.Background()
	var seenDraft map[string]any
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(draftJSON, &seenDraft); err != nil {
				t.Fatalf("draft JSON: %v", err)
			}
			return missionRuntimeResultJSON(t, draftJSON, []byte(missionStatusFixtureJSON)), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeMissionClient(runtime, missionRuntimeIdentityClient(t))
	if err != nil {
		t.Fatalf("NewRuntimeMissionClient: %v", err)
	}

	status, err := client.Track(ctx, MissionTrackRequest{
		MissionCarrierBase: baseMissionCarrier(),
		MissionID:          "2026-07-04_010203_weather",
	})
	if err != nil {
		t.Fatalf("Track: %v", err)
	}

	if status.MissionID != "2026-07-04_010203_weather" || !status.Terminal {
		t.Fatalf("unexpected status: %#v", status)
	}
	if seenDraft["caller_ura"] != "easynet:///r/example/agent/alice.sdk" ||
		seenDraft["callee_ura"] != "easynet:///r/example/device/dev-a" ||
		seenDraft["subject_ura"] != "easynet:///r/example/device/dev-a" ||
		seenDraft["nonce_base64"] != "AQIDBAUGBwgJCgsMDQ4PEA==" {
		t.Fatalf("incomplete Mission runtime draft: %#v", seenDraft)
	}
	if seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.mission.track@1.0.0" {
		t.Fatalf("unexpected descriptor ref: %#v", seenDraft)
	}
	args, ok := seenDraft["args"].(map[string]any)
	if !ok || args["run_id"] != "2026-07-04_010203_weather" {
		t.Fatalf("unexpected args: %#v", seenDraft["args"])
	}
}

func TestMissionRuntimeTransportRejectsLegacyStatusOutput(t *testing.T) {
	ctx := context.Background()
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			return missionRuntimeResultJSON(t, draftJSON, []byte(`{
				"run_id": "2026-07-04_010203_weather",
				"run_dir": "/tmp/easynet/runs/2026-07-04_010203_weather",
				"outputs": {"weather": {"ok": true}},
				"meta": {
					"trace_id": "2026-07-04_010203_weather",
					"status": "partial",
					"steps_failed": 1
				}
			}`)), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeMissionClient(runtime, missionRuntimeIdentityClient(t))
	if err != nil {
		t.Fatalf("NewRuntimeMissionClient: %v", err)
	}

	_, err = client.Track(ctx, MissionTrackRequest{
		MissionCarrierBase: baseMissionCarrier(),
		MissionID:          "2026-07-04_010203_weather",
	})
	if err == nil {
		t.Fatal("Track accepted legacy mission output; want schema-backed MissionStatus rejection")
	}
}

func TestMissionRuntimeTransportBuildsRunFileInvocation(t *testing.T) {
	ctx := context.Background()
	sourcePath := filepath.Join(t.TempDir(), "demo.eal")
	if err := os.WriteFile(sourcePath, []byte("mission weather\nlet r = local.observe_health()\n"), 0o600); err != nil {
		t.Fatalf("write source: %v", err)
	}
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeMissionClient(runtime, missionRuntimeIdentityClient(t))
	if err != nil {
		t.Fatalf("NewRuntimeMissionClient: %v", err)
	}
	draft, err := client.BuildRunFileInvocation(ctx, MissionRunFileRequest{
		MissionCarrierBase: baseMissionCarrier(),
		Path:               sourcePath,
	})
	if err != nil {
		t.Fatalf("BuildRunFileInvocation: %v", err)
	}
	args := draft.JSONArgs().(map[string]any)
	if args["source"] != "mission weather\nlet r = local.observe_health()\n" || args["label"] != sourcePath {
		t.Fatalf("run-file args = %#v", args)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.mission.run@1.0.0" {
		t.Fatalf("descriptor_ref = %q", draft.DescriptorRef())
	}
}

func TestMissionRuntimeTransportOpensEventStreamThroughRuntimeCore(t *testing.T) {
	ctx := context.Background()
	var seenDraft map[string]any
	closeCalls := 0
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		OpenStreamFunc: func(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error) {
			if err := json.Unmarshal(draftJSON, &seenDraft); err != nil {
				t.Fatalf("draft JSON: %v", err)
			}
			return StreamTransportFunc{
				RecvFunc: func(ctx context.Context) ([]byte, error) {
					return []byte(`{
						"sequence": 1,
						"kind": "data",
						"state": "Open",
						"payload_json": {
							"profile": "mission",
							"kind": "mission_event",
							"mission_id": "2026-07-04_010203_weather",
							"sequence": 4,
							"event_type": "progress",
							"occurred_unix_ms": 1004,
							"terminal": false,
							"payload": {"delta": "runtime"},
							"receipt": {},
							"metadata": {}
						}
					}`), nil
				},
				CloseFunc: func(ctx context.Context) error {
					closeCalls++
					return nil
				},
			}, []byte(`{"stream_id":"mission-events-runtime","state":"Open"}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeMissionClient(runtime, missionRuntimeIdentityClient(t))
	if err != nil {
		t.Fatalf("NewRuntimeMissionClient: %v", err)
	}

	stream, err := client.OpenEventStream(ctx, MissionEventListRequest{
		MissionCarrierBase: baseMissionCarrier(),
		MissionID:          "2026-07-04_010203_weather",
		CursorSequence:     4,
	})
	if err != nil {
		t.Fatalf("OpenEventStream: %v", err)
	}
	event, err := stream.Next(ctx)
	if err != nil {
		t.Fatalf("Next: %v", err)
	}
	if event.EventType != "progress" || stream.StreamID() != "mission-events-runtime" {
		t.Fatalf("unexpected stream event: stream=%q event=%#v", stream.StreamID(), event)
	}
	if seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.mission.events@1.0.0" {
		t.Fatalf("unexpected event descriptor ref: %#v", seenDraft)
	}
	if err := stream.Close(ctx); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if closeCalls != 1 {
		t.Fatalf("close calls = %d, want 1", closeCalls)
	}
}

func missionRuntimeIdentityClient(t *testing.T) *IdentityClient {
	t.Helper()
	client, err := NewIdentityClient(IdentityTransportFunc{
		BuildURAFunc: func(ctx context.Context, requestJSON []byte) ([]byte, error) {
			var req URABuildRequest
			if err := json.Unmarshal(requestJSON, &req); err != nil {
				t.Fatalf("BuildURA request: %v", err)
			}
			owner := strings.TrimPrefix(req.OwnerURA, "easynet:///r/example/device/")
			return []byte(`{
				"kind": "ability",
				"valid": true,
				"ura": "easynet:///r/example/ability/device.` + owner + "." + req.AbilityName + `",
				"profile": "directory_identity",
				"components": {},
				"metadata": {}
			}`), nil
		},
		BuildDescriptorRefFunc: func(ctx context.Context, requestJSON []byte) ([]byte, error) {
			var req DescriptorRefBuildRequest
			if err := json.Unmarshal(requestJSON, &req); err != nil {
				t.Fatalf("BuildDescriptorRef request: %v", err)
			}
			return []byte(`{
				"kind": "descriptor_ref",
				"valid": true,
				"descriptor_ref": "` + req.AbilityURA + `@` + req.DescriptorVersion + `",
				"ability_ura": "` + req.AbilityURA + `",
				"descriptor_version": "` + req.DescriptorVersion + `",
				"profile": "directory_identity",
				"components": {},
				"metadata": {}
			}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	return client
}

func missionRuntimeResultJSON(t *testing.T, draftJSON []byte, outputJSON []byte) []byte {
	t.Helper()
	result, err := json.Marshal(map[string]any{
		"ok":                  true,
		"tuple":               json.RawMessage(draftJSON),
		"terminal_state":      "Succeeded",
		"output_content_type": "application/json",
		"output_json":         json.RawMessage(outputJSON),
		"elapsed_ms":          1,
		"receipt":             map[string]any{},
	})
	if err != nil {
		t.Fatalf("marshal runtime result: %v", err)
	}
	return result
}
