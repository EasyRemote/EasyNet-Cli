package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"
)

type memoryMissionTransport struct {
	runInvocationJSON     string
	runFileInvocationJSON string
	trackInvocationJSON   string
	cancelInvocationJSON  string
	statusJSON            string
	eventsJSON            string
	eventsJSONs           []string
	seenRequest           map[string]any
	seenEventRequests     []map[string]any
	closeCalls            int
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
	var seen map[string]any
	_ = json.Unmarshal(requestJSON, &seen)
	m.seenEventRequests = append(m.seenEventRequests, seen)
	if len(m.eventsJSONs) > 0 {
		raw := m.eventsJSONs[0]
		m.eventsJSONs = m.eventsJSONs[1:]
		return []byte(raw), nil
	}
	return []byte(m.eventsJSON), nil
}

func (m *memoryMissionTransport) Close(ctx context.Context) error {
	m.closeCalls++
	return nil
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

func TestMissionEventTailerYieldsUntilTerminal(t *testing.T) {
	transport := newMemoryMissionTransport()
	transport.eventsJSONs = []string{
		missionEventTailPage1JSON,
		missionEventTailPage2JSON,
	}
	client, err := NewMissionClient(transport)
	if err != nil {
		t.Fatalf("NewMissionClient: %v", err)
	}
	tailer, err := client.TailEvents(context.Background(), MissionEventListRequest{
		MissionCarrierBase: baseMissionCarrier(),
		MissionID:          "2026-07-04_010203_weather",
	}, MissionEventTailOptions{Limit: 10})
	if err != nil {
		t.Fatalf("TailEvents: %v", err)
	}

	first, ok, err := tailer.Next(context.Background())
	if err != nil || !ok {
		t.Fatalf("Next first = (%#v, %v, %v)", first, ok, err)
	}
	second, ok, err := tailer.Next(context.Background())
	if err != nil || !ok {
		t.Fatalf("Next second = (%#v, %v, %v)", second, ok, err)
	}
	_, ok, err = tailer.Next(context.Background())
	if err != nil || ok {
		t.Fatalf("Next after terminal = (_, %v, %v), want closed", ok, err)
	}
	if first.EventType != "progress" || second.EventType != "completed" || !tailer.Closed() || tailer.CursorSequence() != 2 {
		t.Fatalf("unexpected tail state: first=%#v second=%#v cursor=%d closed=%v", first, second, tailer.CursorSequence(), tailer.Closed())
	}
	if got := []any{transport.seenEventRequests[0]["cursor_sequence"], transport.seenEventRequests[1]["cursor_sequence"]}; got[0] != float64(0) || got[1] != float64(1) {
		t.Fatalf("cursor sequence requests = %#v", got)
	}
	if transport.seenEventRequests[0]["limit"] != float64(10) {
		t.Fatalf("limit not forwarded: %#v", transport.seenEventRequests[0])
	}
}

func TestMissionEventTailerStopsWithinPageAfterTerminal(t *testing.T) {
	transport := newMemoryMissionTransport()
	transport.eventsJSONs = []string{missionEventTailTerminalThenStrayPageJSON}
	client, err := NewMissionClient(transport)
	if err != nil {
		t.Fatalf("NewMissionClient: %v", err)
	}
	tailer, err := client.TailEvents(context.Background(), MissionEventListRequest{
		MissionCarrierBase: baseMissionCarrier(),
		MissionID:          "2026-07-04_010203_weather",
	}, MissionEventTailOptions{})
	if err != nil {
		t.Fatalf("TailEvents: %v", err)
	}

	first, ok, err := tailer.Next(context.Background())
	if err != nil || !ok {
		t.Fatalf("Next terminal = (%#v, %v, %v)", first, ok, err)
	}
	second, ok, err := tailer.Next(context.Background())
	if err != nil || ok {
		t.Fatalf("Next after terminal = (%#v, %v, %v), want closed", second, ok, err)
	}
	if first.EventType != "completed" || !tailer.Closed() {
		t.Fatalf("terminal tail state = event=%#v closed=%v", first, tailer.Closed())
	}
}

func TestMissionPlanRendersEALAndChildIntents(t *testing.T) {
	plan, err := NewMissionPlanWithOptions("nightly", "go-test", "v1")
	if err != nil {
		t.Fatalf("NewMissionPlanWithOptions: %v", err)
	}
	fetch, err := plan.Step("er.fetch", MissionPlanStepOptions{Args: map[string]any{"url": "https://example.test/data.json"}})
	if err != nil {
		t.Fatalf("fetch step: %v", err)
	}
	retries := 2
	timeout := 1.2
	if _, err := plan.Step("er.summarize", MissionPlanStepOptions{
		Args:           map[string]any{"rows": fetch.Output()},
		Retries:        &retries,
		TimeoutSeconds: &timeout,
		OnFailure:      "retry",
	}); err != nil {
		t.Fatalf("summarize step: %v", err)
	}

	eal, err := plan.ToEAL()
	if err != nil {
		t.Fatalf("ToEAL: %v", err)
	}
	for _, want := range []string{
		`// generated by easynet daemon sdk v1`,
		`// created_by: go-test`,
		`mission "nightly" {`,
		`let fetch = call "er.fetch" with { url = "https://example.test/data.json" }`,
		`let summarize = call "er.summarize" with { rows = fetch.output } timeout 2 retries 2 on_failure retry`,
	} {
		if !strings.Contains(eal, want) {
			t.Fatalf("EAL missing %q:\n%s", want, eal)
		}
	}
	intents := plan.ChildInvocationIntents()
	if len(intents) != 2 || intents[0].StepID != "fetch" || intents[1].Ability != "er.summarize" || intents[1].OnFailure != "retry" {
		t.Fatalf("unexpected intents: %#v", intents)
	}
}

func TestMissionPlanRejectsInvalidFields(t *testing.T) {
	plan, err := NewMissionPlan("p")
	if err != nil {
		t.Fatalf("NewMissionPlan: %v", err)
	}
	timeout := 0.0
	if _, err := plan.Step("er.fn", MissionPlanStepOptions{TimeoutSeconds: &timeout}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("timeout error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := plan.Step("er.fn", MissionPlanStepOptions{Args: map[string]any{"payload": map[string]any{"nested": 1}}}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("structured field error = %v, want %s", err, ErrInvalidArgument)
	}
	foreign, err := NewMissionPlan("other")
	if err != nil {
		t.Fatalf("NewMissionPlan foreign: %v", err)
	}
	foreignStep, err := foreign.Step("er.src", MissionPlanStepOptions{})
	if err != nil {
		t.Fatalf("foreign step: %v", err)
	}
	_, err = plan.Step("er.fn", MissionPlanStepOptions{Args: map[string]any{"data": foreignStep.Output()}})
	if err == nil {
		t.Fatal("foreign step output accepted")
	}
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) || sdkErr.Details["reason"] != "foreign_step_output" {
		t.Fatalf("foreign output error details = %#v", err)
	}
}

func TestMissionPlanValidatesChildInvocationFacts(t *testing.T) {
	plan, err := NewMissionPlan("nightly")
	if err != nil {
		t.Fatalf("NewMissionPlan: %v", err)
	}
	if _, err := plan.Step("observe.health", MissionPlanStepOptions{}); err != nil {
		t.Fatalf("Step: %v", err)
	}
	status, err := NewMissionStatusFromJSON([]byte(strings.ReplaceAll(missionStatusFixtureJSON, `"step_id": "s1"`, `"step_id": "health"`)))
	if err != nil {
		t.Fatalf("NewMissionStatusFromJSON: %v", err)
	}

	conformance, err := plan.ValidateChildInvocations(status)
	if err != nil {
		t.Fatalf("ValidateChildInvocations: %v", err)
	}
	if !conformance.Passed() || len(conformance.ExpectedSteps) != 1 || conformance.ReceiptBackedSteps[0] != "health" {
		t.Fatalf("unexpected conformance: %#v", conformance)
	}

	missing, err := NewMissionPlan("nightly")
	if err != nil {
		t.Fatalf("NewMissionPlan missing: %v", err)
	}
	if _, err := missing.Step("observe.health", MissionPlanStepOptions{}); err != nil {
		t.Fatalf("missing health step: %v", err)
	}
	if _, err := missing.Step("notify.user", MissionPlanStepOptions{}); err != nil {
		t.Fatalf("missing notify step: %v", err)
	}
	_, err = missing.ValidateChildInvocations(status)
	if err == nil || !IsCode(err, ErrProtocol) {
		t.Fatalf("missing child facts error = %v, want %s", err, ErrProtocol)
	}
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) || sdkErr.Details["reason"] != "mission_child_invocation_mismatch" {
		t.Fatalf("unexpected mismatch error details: %#v", err)
	}

	mismatchedStatus, err := NewMissionStatusFromJSON([]byte(strings.ReplaceAll(
		strings.ReplaceAll(missionStatusFixtureJSON, `"step_id": "s1"`, `"step_id": "health"`),
		`"ability": "observe.health"`,
		`"ability": "observe.other"`,
	)))
	if err != nil {
		t.Fatalf("mismatched status: %v", err)
	}
	_, err = plan.ValidateChildInvocations(mismatchedStatus)
	if err == nil {
		t.Fatalf("ability mismatch accepted")
	}
	if !errors.As(err, &sdkErr) {
		t.Fatalf("ability mismatch error type = %T", err)
	}
	mismatchedSteps, ok := sdkErr.Details["ability_mismatched_steps"].([]string)
	if !ok || len(mismatchedSteps) != 1 || mismatchedSteps[0] != "health" {
		t.Fatalf("ability mismatch details = %#v", sdkErr.Details)
	}

	incompleteStatus := status
	incompleteChild := incompleteStatus.ChildInvocations[0]
	incompleteChild.InvocationURA = nil
	incompleteStatus.ChildInvocations = []MissionChildInvocation{incompleteChild}
	_, err = plan.ValidateChildInvocations(incompleteStatus)
	if err == nil || !errors.As(err, &sdkErr) {
		t.Fatalf("incomplete child fact error = %#v", err)
	}
	incompleteSteps, ok := sdkErr.Details["incomplete_fact_steps"].([]string)
	if !ok || len(incompleteSteps) != 1 || incompleteSteps[0] != "health" {
		t.Fatalf("incomplete fact details = %#v", sdkErr.Details)
	}
}

func TestMissionStatusRejectsIncompleteChildInvocationFact(t *testing.T) {
	_, err := NewMissionStatusFromJSON([]byte(strings.ReplaceAll(
		missionStatusFixtureJSON,
		`"request_id": "req-1"`,
		`"request_id": null`,
	)))
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("incomplete child invocation fact error = %v, want %s", err, ErrInvalidArgument)
	}

	_, err = NewMissionStatusFromJSON([]byte(strings.ReplaceAll(
		missionStatusFixtureJSON,
		`"receipt_hash": "bbbb"`,
		`"receipt_hash": ""`,
	)))
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("incomplete child invocation receipt fact error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestMissionEventTailerReportsDroppedEvents(t *testing.T) {
	transport := newMemoryMissionTransport()
	transport.eventsJSONs = []string{missionEventDroppedPageJSON}
	client, err := NewMissionClient(transport)
	if err != nil {
		t.Fatalf("NewMissionClient: %v", err)
	}
	tailer, err := client.TailEvents(context.Background(), MissionEventListRequest{
		MissionCarrierBase: baseMissionCarrier(),
		MissionID:          "2026-07-04_010203_weather",
	}, MissionEventTailOptions{})
	if err != nil {
		t.Fatalf("TailEvents: %v", err)
	}

	_, _, err = tailer.Next(context.Background())
	if !IsCode(err, ErrProtocol) {
		t.Fatalf("dropped tail error = %v, want %s", err, ErrProtocol)
	}
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) || sdkErr.Details["reason"] != "mission_events_dropped" {
		t.Fatalf("missing dropped-event details: %#v", err)
	}
}

func TestMissionEventTailerRejectsNoCursorProgress(t *testing.T) {
	transport := newMemoryMissionTransport()
	transport.eventsJSONs = []string{missionEventNoProgressPageJSON}
	client, err := NewMissionClient(transport)
	if err != nil {
		t.Fatalf("NewMissionClient: %v", err)
	}
	tailer, err := client.TailEvents(context.Background(), MissionEventListRequest{
		MissionCarrierBase: baseMissionCarrier(),
		MissionID:          "2026-07-04_010203_weather",
	}, MissionEventTailOptions{})
	if err != nil {
		t.Fatalf("TailEvents: %v", err)
	}
	if _, _, err := tailer.Next(context.Background()); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("no-progress error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestMissionEventTailerEmptyPageBudgetAndValidation(t *testing.T) {
	transport := newMemoryMissionTransport()
	transport.eventsJSONs = []string{missionEventEmptyPageJSON, missionEventEmptyPageJSON}
	client, err := NewMissionClient(transport)
	if err != nil {
		t.Fatalf("NewMissionClient: %v", err)
	}
	sleeps := 0
	tailer, err := client.TailEvents(context.Background(), MissionEventListRequest{
		MissionCarrierBase: baseMissionCarrier(),
		MissionID:          "2026-07-04_010203_weather",
	}, MissionEventTailOptions{
		MaxEmptyPages:  1,
		PollInterval:   time.Millisecond,
		Sleep:          func(context.Context, time.Duration) error { sleeps++; return nil },
		CursorSequence: 9,
	})
	if err != nil {
		t.Fatalf("TailEvents: %v", err)
	}
	if _, ok, err := tailer.Next(context.Background()); err != nil || ok {
		t.Fatalf("empty budget Next = (_, %v, %v), want stop", ok, err)
	}
	if !tailer.Closed() || sleeps != 1 {
		t.Fatalf("empty budget state: closed=%v sleeps=%d", tailer.Closed(), sleeps)
	}
	if transport.seenEventRequests[0]["cursor_sequence"] != float64(9) {
		t.Fatalf("initial cursor not forwarded: %#v", transport.seenEventRequests[0])
	}
	if _, err := client.TailEvents(context.Background(), MissionEventListRequest{
		MissionCarrierBase: baseMissionCarrier(),
		MissionID:          "2026-07-04_010203_weather",
	}, MissionEventTailOptions{Limit: 1001}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("invalid tail options error = %v, want %s", err, ErrInvalidArgument)
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

func TestMissionClientCloseDelegatesOnceAndFailsClosed(t *testing.T) {
	transport := newMemoryMissionTransport()
	client, err := NewMissionClient(transport)
	if err != nil {
		t.Fatalf("NewMissionClient: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if transport.closeCalls != 1 {
		t.Fatalf("close calls = %d, want 1", transport.closeCalls)
	}
	_, err = client.BuildRunEALInvocation(context.Background(), MissionRunRequest{
		MissionCarrierBase: baseMissionCarrier(),
		Source:             "mission weather\nlet r = local.observe_health()",
		Label:              "weather",
	})
	if err == nil {
		t.Fatalf("BuildRunEALInvocation after close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if transport.seenRequest != nil {
		t.Fatalf("transport called after close: %#v", transport.seenRequest)
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

const missionEventTailPage1JSON = `{
  "profile": "mission",
  "kind": "mission_event_page",
  "mission_id": "2026-07-04_010203_weather",
  "cursor_sequence": 0,
  "next_cursor_sequence": 1,
  "has_more": true,
  "dropped_count": 0,
  "events": [
    {
      "profile": "mission",
      "kind": "mission_event",
      "mission_id": "2026-07-04_010203_weather",
      "sequence": 0,
      "event_type": "progress",
      "occurred_unix_ms": 1000,
      "terminal": false,
      "payload": {"delta": "hello"},
      "receipt": {},
      "metadata": {"step_id": "s1"}
    }
  ],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}`

const missionEventTailPage2JSON = `{
  "profile": "mission",
  "kind": "mission_event_page",
  "mission_id": "2026-07-04_010203_weather",
  "cursor_sequence": 1,
  "next_cursor_sequence": 2,
  "has_more": false,
  "dropped_count": 0,
  "events": [
    {
      "profile": "mission",
      "kind": "mission_event",
      "mission_id": "2026-07-04_010203_weather",
      "sequence": 1,
      "event_type": "completed",
      "occurred_unix_ms": 1001,
      "terminal": true,
      "payload": {"reply": "done"},
      "receipt": {"receipt_ura": "easynet:///r/example/receipt/terminal"},
      "metadata": {}
    }
  ],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}`

const missionEventTailTerminalThenStrayPageJSON = `{
  "profile": "mission",
  "kind": "mission_event_page",
  "mission_id": "2026-07-04_010203_weather",
  "cursor_sequence": 0,
  "next_cursor_sequence": 2,
  "has_more": false,
  "dropped_count": 0,
  "events": [
    {
      "profile": "mission",
      "kind": "mission_event",
      "mission_id": "2026-07-04_010203_weather",
      "sequence": 0,
      "event_type": "completed",
      "occurred_unix_ms": 1000,
      "terminal": true,
      "payload": {"reply": "done"},
      "receipt": {"receipt_ura": "easynet:///r/example/receipt/terminal"},
      "metadata": {}
    },
    {
      "profile": "mission",
      "kind": "mission_event",
      "mission_id": "2026-07-04_010203_weather",
      "sequence": 1,
      "event_type": "progress",
      "occurred_unix_ms": 1001,
      "terminal": false,
      "payload": {"delta": "stray"},
      "receipt": {},
      "metadata": {}
    }
  ],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}`

const missionEventDroppedPageJSON = `{
  "profile": "mission",
  "kind": "mission_event_page",
  "mission_id": "2026-07-04_010203_weather",
  "cursor_sequence": 0,
  "next_cursor_sequence": 3,
  "has_more": false,
  "dropped_count": 2,
  "events": [],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}`

const missionEventNoProgressPageJSON = `{
  "profile": "mission",
  "kind": "mission_event_page",
  "mission_id": "2026-07-04_010203_weather",
  "cursor_sequence": 0,
  "next_cursor_sequence": 0,
  "has_more": true,
  "dropped_count": 0,
  "events": [],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}`

const missionEventEmptyPageJSON = `{
  "profile": "mission",
  "kind": "mission_event_page",
  "mission_id": "2026-07-04_010203_weather",
  "cursor_sequence": 9,
  "next_cursor_sequence": 9,
  "has_more": false,
  "dropped_count": 0,
  "events": [],
  "metadata": {"profile": "mission", "carrier_owner": "daemon_sdk"}
}`
