package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryHostBindingTransport struct {
	bindingJSON  string
	requestJSON  string
	itemJSON     string
	errorJSON    string
	terminalJSON string
	hashJSON     string
	seenRequest  map[string]any
	closeCalls   int
}

func (m *memoryHostBindingTransport) remember(requestJSON []byte) {
	_ = json.Unmarshal(requestJSON, &m.seenRequest)
}

func (m *memoryHostBindingTransport) BuildHostStreamBinding(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.bindingJSON), nil
}

func (m *memoryHostBindingTransport) DecodeRequest(ctx context.Context, envelopeJSON []byte) ([]byte, error) {
	m.remember(envelopeJSON)
	return []byte(m.requestJSON), nil
}

func (m *memoryHostBindingTransport) EncodeItem(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.itemJSON), nil
}

func (m *memoryHostBindingTransport) EncodeError(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.errorJSON), nil
}

func (m *memoryHostBindingTransport) EncodeTerminal(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.terminalJSON), nil
}

func (m *memoryHostBindingTransport) FoldOutputHash(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.hashJSON), nil
}

func (m *memoryHostBindingTransport) Close(ctx context.Context) error {
	m.closeCalls++
	return nil
}

func newMemoryHostBindingTransport() *memoryHostBindingTransport {
	return &memoryHostBindingTransport{
		bindingJSON:  hostStreamBindingFixtureJSON,
		requestJSON:  hostStreamRequestFixtureJSON,
		itemJSON:     hostStreamItemFrameFixtureJSON,
		errorJSON:    hostStreamErrorFrameFixtureJSON,
		terminalJSON: hostStreamTerminalFrameFixtureJSON,
		hashJSON:     hostStreamHashStateFixtureJSON,
	}
}

func TestHostBindingBuildBindingAndDecodeRequest(t *testing.T) {
	transport := newMemoryHostBindingTransport()
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
		Metadata:      map[string]any{"owner": "easyremote"},
	})
	if err != nil {
		t.Fatalf("BuildHostStreamBinding: %v", err)
	}
	if binding.FrameSchema != hostStreamFrameSchema || binding.Lifecycle["frame_contract_owner"] != "daemon_sdk" {
		t.Fatalf("unexpected binding: %#v", binding)
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
		t.Fatalf("DecodeRequest: %v", err)
	}
	if request.Function != "weather.stream" || request.Metadata["wire"] != "host_stream_request_v1" {
		t.Fatalf("unexpected request: %#v", request)
	}
}

func TestHostBindingRejectsRelativeEndpointAndSchemaDrift(t *testing.T) {
	client, err := NewHostBindingClient(newMemoryHostBindingTransport())
	if err != nil {
		t.Fatalf("NewHostBindingClient: %v", err)
	}
	req := HostStreamBindingRequest{
		BindingID:     "binding-weather-1",
		DescriptorRef: "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
		Endpoint:      "tmp/easynet-weather.sock",
		FrameSchema:   hostStreamFrameSchema,
	}
	if _, err := client.BuildHostStreamBinding(context.Background(), req); err == nil {
		t.Fatalf("relative endpoint accepted")
	}
	req.Endpoint = "/tmp/easynet-weather.sock"
	req.FrameSchema = "other.schema.json"
	if _, err := client.BuildHostStreamBinding(context.Background(), req); err == nil {
		t.Fatalf("schema drift accepted")
	}
}

func TestHostBindingEncodesFrameVariants(t *testing.T) {
	transport := newMemoryHostBindingTransport()
	client, err := NewHostBindingClient(transport)
	if err != nil {
		t.Fatalf("NewHostBindingClient: %v", err)
	}

	item, err := client.EncodeItem(context.Background(), 0, map[string]any{"token": "hello"})
	if err != nil {
		t.Fatalf("EncodeItem: %v", err)
	}
	if item.FrameType != "item" || item.Seq == nil || *item.Seq != 0 || item.Value == nil {
		t.Fatalf("unexpected item frame: %#v", item)
	}

	errorFrame, err := client.EncodeError(context.Background(), &SDKError{
		Code:    ErrInvalidArgument,
		Stage:   "host",
		Retry:   RetryNever,
		Message: "bad input",
		Details: map[string]any{},
	})
	if err != nil {
		t.Fatalf("EncodeError: %v", err)
	}
	if errorFrame.FrameType != "error" || errorFrame.Error == nil || errorFrame.Error.Code != ErrInvalidArgument {
		t.Fatalf("unexpected error frame: %#v", errorFrame)
	}

	terminal, err := client.EncodeTerminal(context.Background(), HostStreamTerminalSummary{
		OutputHash: "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15",
		Frames:     1,
	})
	if err != nil {
		t.Fatalf("EncodeTerminal: %v", err)
	}
	if terminal.FrameType != "terminal" || terminal.OutputHash == nil || terminal.Terminal == nil {
		t.Fatalf("unexpected terminal frame: %#v", terminal)
	}
}

func TestHostBindingFoldOutputHashRejectsSequenceGap(t *testing.T) {
	transport := newMemoryHostBindingTransport()
	client, err := NewHostBindingClient(transport)
	if err != nil {
		t.Fatalf("NewHostBindingClient: %v", err)
	}
	state := HostStreamHashState{
		Algorithm:  hostStreamHashAlgorithm,
		OutputHash: "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
		Frames:     0,
	}

	folded, err := client.FoldOutputHash(context.Background(), state, 0, map[string]any{"token": "hello"})
	if err != nil {
		t.Fatalf("FoldOutputHash: %v", err)
	}
	if folded.LastSeq == nil || *folded.LastSeq != 0 || folded.CanonicalJSON != `{"token":"hello"}` {
		t.Fatalf("unexpected folded state: %#v", folded)
	}

	if _, err := client.FoldOutputHash(context.Background(), state, 2, map[string]any{"token": "skip"}); err == nil {
		t.Fatalf("hash sequence gap accepted")
	}
}

func TestLocalHostBindingTransportProjectsCodecAndHash(t *testing.T) {
	client, err := NewLocalHostBindingClient(nil)
	if err != nil {
		t.Fatalf("NewLocalHostBindingClient: %v", err)
	}

	binding, err := client.BuildHostStreamBinding(context.Background(), HostStreamBindingRequest{
		BindingID:     "binding-weather-1",
		DescriptorRef: "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
		Endpoint:      "/tmp/easynet-weather.sock",
		FrameSchema:   hostStreamFrameSchema,
		Cleanup:       map[string]any{"mode": "unlink_socket"},
		TimeoutMS:     int64PtrForHostBindingTest(30000),
		Metadata:      map[string]any{"source": "fixture"},
	})
	if err != nil {
		t.Fatalf("BuildHostStreamBinding: %v", err)
	}
	if binding.Lifecycle["frame_contract_owner"] != "daemon_sdk" ||
		binding.Metadata["hash_algorithm"] != hostStreamHashAlgorithm ||
		binding.Readiness["state"] != "declared" {
		t.Fatalf("local binding projection = %#v", binding)
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
		t.Fatalf("DecodeRequest: %v", err)
	}
	if request.Function != "weather.stream" || request.Metadata["frame_contract_owner"] != "daemon_sdk" {
		t.Fatalf("local request projection = %#v", request)
	}

	item, err := client.EncodeItem(context.Background(), 0, map[string]any{"token": "hello"})
	if err != nil {
		t.Fatalf("EncodeItem: %v", err)
	}
	if item.FrameType != "item" || item.Seq == nil || *item.Seq != 0 {
		t.Fatalf("local item frame = %#v", item)
	}

	errorFrame, err := client.EncodeError(context.Background(), &SDKError{
		Code:    ErrInvalidArgument,
		Stage:   "host",
		Retry:   RetryNever,
		Message: "bad input",
		Details: map[string]any{},
	})
	if err != nil {
		t.Fatalf("EncodeError: %v", err)
	}
	if errorFrame.FrameType != "error" || errorFrame.Error == nil || errorFrame.Error.Code != ErrInvalidArgument {
		t.Fatalf("local error frame = %#v", errorFrame)
	}

	state := HostStreamHashState{
		Algorithm:  hostStreamHashAlgorithm,
		OutputHash: "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
		Frames:     0,
	}
	folded, err := client.FoldOutputHash(context.Background(), state, 0, map[string]any{"token": "hello"})
	if err != nil {
		t.Fatalf("FoldOutputHash: %v", err)
	}
	if folded.OutputHash != "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15" ||
		folded.CanonicalJSON != `{"token":"hello"}` {
		t.Fatalf("local folded hash = %#v", folded)
	}

	terminal, err := client.EncodeTerminal(context.Background(), HostStreamTerminalSummary{
		OutputHash: folded.OutputHash,
		Frames:     folded.Frames,
		Metadata:   map[string]any{"canonical_json": folded.CanonicalJSON},
	})
	if err != nil {
		t.Fatalf("EncodeTerminal: %v", err)
	}
	if terminal.FrameType != "terminal" || terminal.OutputHash == nil || *terminal.OutputHash != folded.OutputHash {
		t.Fatalf("local terminal frame = %#v", terminal)
	}
}

func TestHostBindingHashStateRejectsCorruptedFrameCursor(t *testing.T) {
	var zero HostStreamHashState
	if err := json.Unmarshal([]byte(hostStreamHashStateFixtureJSON), &zero); err != nil {
		t.Fatalf("unmarshal fixture: %v", err)
	}
	var lastSeq uint64
	zero.Frames = 0
	zero.LastSeq = &lastSeq
	raw, _ := json.Marshal(zero)
	if _, err := NewHostStreamHashStateFromJSON(raw); err == nil {
		t.Fatalf("accepted zero-frame state with last_seq")
	} else if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}

	var gap HostStreamHashState
	if err := json.Unmarshal([]byte(hostStreamHashStateFixtureJSON), &gap); err != nil {
		t.Fatalf("unmarshal fixture: %v", err)
	}
	gap.Frames = 3
	gap.LastSeq = &lastSeq
	raw, _ = json.Marshal(gap)
	if _, err := NewHostStreamHashStateFromJSON(raw); err == nil {
		t.Fatalf("accepted state whose last_seq does not match frames")
	} else if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestHostBindingFoldOutputHashRejectsCorruptedLocalState(t *testing.T) {
	transport := newMemoryHostBindingTransport()
	client, err := NewHostBindingClient(transport)
	if err != nil {
		t.Fatalf("NewHostBindingClient: %v", err)
	}
	lastSeq := uint64(0)
	state := HostStreamHashState{
		Algorithm:  hostStreamHashAlgorithm,
		OutputHash: "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
		Frames:     2,
		LastSeq:    &lastSeq,
	}

	if _, err := client.FoldOutputHash(context.Background(), state, 2, map[string]any{"token": "late"}); err == nil {
		t.Fatalf("accepted corrupted local hash state")
	} else if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if transport.seenRequest != nil {
		t.Fatalf("transport called for corrupted state: %#v", transport.seenRequest)
	}
}

func TestHostBindingClientCloseDelegatesOnceAndFailsClosed(t *testing.T) {
	transport := newMemoryHostBindingTransport()
	client, err := NewHostBindingClient(transport)
	if err != nil {
		t.Fatalf("NewHostBindingClient: %v", err)
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
	_, err = client.EncodeItem(context.Background(), 0, map[string]any{"token": "hello"})
	if err == nil {
		t.Fatalf("EncodeItem after close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if transport.seenRequest != nil {
		t.Fatalf("transport called after close: %#v", transport.seenRequest)
	}
}

func int64PtrForHostBindingTest(value int64) *int64 {
	return &value
}

const hostStreamBindingFixtureJSON = `{
  "binding_id": "binding-weather-1",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
  "endpoint": "/tmp/easynet-weather.sock",
  "frame_schema": "host-stream-frame.schema.json",
  "cleanup": {"mode": "unlink_socket"},
  "timeout_ms": 30000,
  "readiness": {"state": "declared", "checked": false, "endpoint_ready": null},
  "lifecycle": {
    "endpoint_owner": "product_host",
    "process_owner": "product_host",
    "frame_contract_owner": "daemon_sdk"
  },
  "metadata": {
    "profile": "host_binding",
    "source": "fixture",
    "frame_schema": "host-stream-frame.schema.json",
    "hash_algorithm": "sha256(prev_hash || seq_be || canonical_json(value))"
  }
}`

const hostStreamRequestFixtureJSON = `{
  "function": "weather.stream",
  "args": {"city": "Singapore"},
  "call_id": "call-weather-1",
  "caller": "easynet:///r/example/user/alice",
  "metadata": {"wire": "host_stream_request_v1", "source": "fixture"}
}`

const hostStreamItemFrameFixtureJSON = `{
  "frame_type": "item",
  "seq": 0,
  "value": {"token": "hello"},
  "error": null,
  "terminal": null,
  "output_hash": null
}`

const hostStreamErrorFrameFixtureJSON = `{
  "frame_type": "error",
  "seq": null,
  "value": null,
  "error": {
    "code": "InvalidArgument",
    "stage": "host",
    "message": "bad input",
    "retry": "never",
    "details": {}
  },
  "terminal": null,
  "output_hash": null
}`

const hostStreamTerminalFrameFixtureJSON = `{
  "frame_type": "terminal",
  "seq": 1,
  "value": null,
  "error": null,
  "terminal": {
    "output_hash": "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15",
    "frames": 1,
    "metadata": {"canonical_json": "{\"token\":\"hello\"}"}
  },
  "output_hash": "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15"
}`

const hostStreamHashStateFixtureJSON = `{
  "algorithm": "sha256(prev_hash || seq_be || canonical_json(value))",
  "output_hash": "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15",
  "frames": 1,
  "last_seq": 0,
  "canonical_json": "{\"token\":\"hello\"}"
}`
