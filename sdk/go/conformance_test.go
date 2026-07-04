package easynet

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

const sharedConformanceFixtureRoot = "sdk/conformance/fixtures"

func TestGoFacadeConsumesSharedConformanceFixtures(t *testing.T) {
	root := repositoryRoot(t)

	draft, err := NewInvocationDraftFromJSON(sharedFixture(t, root, "invocation.complete.v4.json"))
	if err != nil {
		t.Fatalf("NewInvocationDraftFromJSON(shared fixture): %v", err)
	}
	if draft.CallerURA() != "easynet:///r/example/agent/alice.sdk" || !draft.HasJSONArgs() {
		t.Fatalf("unexpected invocation draft from shared fixture: %#v", draft)
	}

	prepared, err := NewPreparedInvocationFromJSON(sharedFixture(t, root, "prepared.signing-material.v4.json"))
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON(shared fixture): %v", err)
	}
	if prepared.SubmitReady() || prepared.SigningMaterial().Algorithm() != "ed25519" {
		t.Fatalf("unexpected prepared invocation from shared fixture: %#v", prepared)
	}

	runtimeErr, err := DecodeDaemonErrorJSON(sharedFixture(t, root, "runtime.error.v4.json"))
	if err != nil {
		t.Fatalf("DecodeDaemonErrorJSON(shared fixture): %v", err)
	}
	if runtimeErr == nil || runtimeErr.Code != ErrInvalidArgument || runtimeErr.Retry != RetryNever {
		t.Fatalf("unexpected runtime error from shared fixture: %#v", runtimeErr)
	}

	health, err := NewRuntimeHealthFromJSON(sharedFixture(t, root, "health.ready.v4.json"))
	if err != nil {
		t.Fatalf("NewRuntimeHealthFromJSON(shared fixture): %v", err)
	}
	if !health.APIAlive() || !health.Ready() {
		t.Fatalf("unexpected runtime health from shared fixture: %#v", health)
	}
}

func TestGoHostBindingFacadeConsumesSharedConformanceFixtures(t *testing.T) {
	root := repositoryRoot(t)
	transport := &sharedHostBindingTransport{
		bindingJSON:  sharedFixture(t, root, "host-stream-binding.v4.json"),
		requestJSON:  sharedFixture(t, root, "host-stream-request.v4.json"),
		itemJSON:     sharedFixture(t, root, "host-stream-frame.v4.json"),
		terminalJSON: sharedTerminalFrameFixture(t, root),
		hashJSON:     sharedFixture(t, root, "host-stream-hash-state.v4.json"),
	}
	client, err := NewHostBindingClient(transport)
	if err != nil {
		t.Fatalf("NewHostBindingClient: %v", err)
	}

	binding, err := client.BuildHostStreamBinding(context.Background(), HostStreamBindingRequest{
		BindingID:     "binding-weather-1",
		DescriptorRef: "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
		Endpoint:      "/tmp/easynet-weather.sock",
		FrameSchema:   hostStreamFrameSchema,
		Cleanup:       map[string]any{"mode": "unlink_socket"},
	})
	if err != nil {
		t.Fatalf("BuildHostStreamBinding(shared fixture): %v", err)
	}
	if binding.BindingID != "binding-weather-1" || binding.Lifecycle["frame_contract_owner"] != "daemon_sdk" {
		t.Fatalf("unexpected binding from shared fixture: %#v", binding)
	}

	request, err := client.DecodeRequest(context.Background(), HostStreamEnvelope{
		Request: HostStreamEnvelopeRequest{
			Fn:     "weather.stream",
			Args:   map[string]any{"city": "Singapore"},
			CallID: "call-weather-1",
			Caller: "easynet:///r/example/user/alice",
		},
	})
	if err != nil {
		t.Fatalf("DecodeRequest(shared fixture): %v", err)
	}
	if request.Function != "weather.stream" || request.Metadata["source"] != "fixture" {
		t.Fatalf("unexpected request from shared fixture: %#v", request)
	}

	item, err := client.EncodeItem(context.Background(), 0, map[string]any{"token": "hello"})
	if err != nil {
		t.Fatalf("EncodeItem(shared fixture): %v", err)
	}
	if item.FrameType != "item" || item.Seq == nil || *item.Seq != 0 {
		t.Fatalf("unexpected item frame from shared fixture: %#v", item)
	}

	terminal, err := client.EncodeTerminal(context.Background(), HostStreamTerminalSummary{
		OutputHash: "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15",
		Frames:     1,
	})
	if err != nil {
		t.Fatalf("EncodeTerminal(shared fixture): %v", err)
	}
	if terminal.Terminal == nil || terminal.OutputHash == nil {
		t.Fatalf("unexpected terminal frame from shared fixture: %#v", terminal)
	}

	state := HostStreamHashState{
		Algorithm:  hostStreamHashAlgorithm,
		OutputHash: "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
		Frames:     0,
	}
	folded, err := client.FoldOutputHash(context.Background(), state, 0, map[string]any{"token": "hello"})
	if err != nil {
		t.Fatalf("FoldOutputHash(shared fixture): %v", err)
	}
	if folded.LastSeq == nil || *folded.LastSeq != 0 || folded.CanonicalJSON != `{"token":"hello"}` {
		t.Fatalf("unexpected hash state from shared fixture: %#v", folded)
	}
}

type sharedHostBindingTransport struct {
	bindingJSON  []byte
	requestJSON  []byte
	itemJSON     []byte
	terminalJSON []byte
	hashJSON     []byte
}

func (t *sharedHostBindingTransport) BuildHostStreamBinding(context.Context, []byte) ([]byte, error) {
	return t.bindingJSON, nil
}

func (t *sharedHostBindingTransport) DecodeRequest(context.Context, []byte) ([]byte, error) {
	return t.requestJSON, nil
}

func (t *sharedHostBindingTransport) EncodeItem(context.Context, []byte) ([]byte, error) {
	return t.itemJSON, nil
}

func (t *sharedHostBindingTransport) EncodeError(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedHostBindingTransport) EncodeTerminal(context.Context, []byte) ([]byte, error) {
	return t.terminalJSON, nil
}

func (t *sharedHostBindingTransport) FoldOutputHash(context.Context, []byte) ([]byte, error) {
	return t.hashJSON, nil
}

func (t *sharedHostBindingTransport) Close(context.Context) error {
	return nil
}

func repositoryRoot(t *testing.T) string {
	t.Helper()
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("locate test file")
	}
	return filepath.Clean(filepath.Join(filepath.Dir(file), "..", ".."))
}

func sharedFixture(t *testing.T, root, name string) []byte {
	t.Helper()
	path := filepath.Join(root, sharedConformanceFixtureRoot, name)
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read shared fixture %s: %v", path, err)
	}
	return raw
}

func sharedTerminalFrameFixture(t *testing.T, root string) []byte {
	t.Helper()
	var summary HostStreamTerminalSummary
	if err := json.Unmarshal(sharedFixture(t, root, "host-stream-terminal.v4.json"), &summary); err != nil {
		t.Fatalf("decode shared terminal summary fixture: %v", err)
	}
	raw, err := json.Marshal(map[string]any{
		"frame_type":  "terminal",
		"seq":         uint64(summary.Frames),
		"value":       nil,
		"error":       nil,
		"terminal":    summary,
		"output_hash": summary.OutputHash,
	})
	if err != nil {
		t.Fatalf("build shared terminal frame fixture: %v", err)
	}
	return raw
}
