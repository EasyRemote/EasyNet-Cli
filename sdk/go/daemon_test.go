package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"testing"
)

type testRuntimeHostStartRequest struct {
	payload map[string]any
	err     error
}

func (r testRuntimeHostStartRequest) Validate() error {
	return r.err
}

func (r testRuntimeHostStartRequest) RuntimeHostStartPayload() ([]byte, error) {
	return json.Marshal(r.payload)
}

type memoryDaemonTransport struct {
	discoverJSON string
	startJSON    string
	attachJSON   string
	statusJSON   string
	stopJSON     string
	startCalls   int
	stopCalls    int
	detachCalls  int
	openCalls    int
	openErr      error
	seenStart    map[string]any
	seenOptions  map[string]any
}

func (m *memoryDaemonTransport) Discover(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(optionsJSON, &m.seenOptions); err != nil {
		return nil, err
	}
	return []byte(m.discoverJSON), nil
}

func (m *memoryDaemonTransport) Start(ctx context.Context, configJSON []byte) ([]byte, error) {
	m.startCalls++
	if err := json.Unmarshal(configJSON, &m.seenStart); err != nil {
		return nil, err
	}
	return []byte(m.startJSON), nil
}

func (m *memoryDaemonTransport) Attach(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if err := json.Unmarshal(optionsJSON, &m.seenOptions); err != nil {
		return nil, err
	}
	return []byte(m.attachJSON), nil
}

func (m *memoryDaemonTransport) Status(ctx context.Context, handleID string) ([]byte, error) {
	return []byte(m.statusJSON), nil
}

func (m *memoryDaemonTransport) OpenRuntime(ctx context.Context, handleID string, optionsJSON []byte) (RuntimeTransport, []byte, error) {
	m.openCalls++
	if err := json.Unmarshal(optionsJSON, &m.seenOptions); err != nil {
		return nil, nil, err
	}
	if m.openErr != nil {
		return nil, nil, m.openErr
	}
	return RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			return []byte(`{"ok":false,"tuple":{},"terminal_state":"Failed","error":{"code":"GENERIC","stage":"runtime"}}`), nil
		},
	}, []byte(`{"ready":true}`), nil
}

func (m *memoryDaemonTransport) Stop(ctx context.Context, handleID string, optionsJSON []byte) ([]byte, error) {
	m.stopCalls++
	return []byte(m.stopJSON), nil
}

func (m *memoryDaemonTransport) Detach(ctx context.Context, handleID string) error {
	m.detachCalls++
	return nil
}

func readyDaemonStatus() string {
	return `{"handle_id":"daemon-1","state":"Running","mode":"authority","pid":42,"endpoints":{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/daemon.sock","public_endpoint":"https://hub.example"}}`
}

func TestRuntimeHostStartReturnsRuntimeReadyHandle(t *testing.T) {
	transport := &memoryDaemonTransport{startJSON: readyDaemonStatus()}

	handle, err := StartRuntimeHost(context.Background(), transport, testRuntimeHostStartRequest{
		payload: map[string]any{
			"mode":       "test-runtime",
			"listen_tcp": "127.0.0.1:9443",
		},
	})
	if err != nil {
		t.Fatalf("StartRuntimeHost: %v", err)
	}

	if handle.HandleID() != "daemon-1" || handle.State() != RuntimeRunning {
		t.Fatalf("unexpected handle: id=%q state=%s", handle.HandleID(), handle.State())
	}
	if transport.seenStart["listen_tcp"] != "127.0.0.1:9443" {
		t.Fatalf("start config not forwarded: %#v", transport.seenStart)
	}
	if handle.Endpoints().InvocationEndpoint != "unix:///tmp/daemon.sock" {
		t.Fatalf("invocation endpoint not preserved: %#v", handle.Endpoints())
	}
}

func TestRuntimeHostStartUsesCanonicalLifecycleTypes(t *testing.T) {
	transport := &memoryDaemonTransport{startJSON: readyDaemonStatus()}
	host, err := NewRuntimeHost(transport)
	if err != nil {
		t.Fatalf("NewRuntimeHost: %v", err)
	}

	handle, err := host.StartRuntime(context.Background(), testRuntimeHostStartRequest{
		payload: map[string]any{"mode": "test-runtime"},
	})
	if err != nil {
		t.Fatalf("RuntimeHost.StartRuntime: %v", err)
	}

	if handle.HandleID() != "daemon-1" || handle.State() != RuntimeRunning {
		t.Fatalf("canonical lifecycle handle mismatch: id=%q state=%s", handle.HandleID(), handle.State())
	}
}

func TestRuntimeHostStartRejectsInvalidRequestBeforeTransport(t *testing.T) {
	transport := &memoryDaemonTransport{startJSON: readyDaemonStatus()}
	requestErr := errors.New("invalid provider request")

	_, err := StartRuntimeHost(context.Background(), transport, testRuntimeHostStartRequest{err: requestErr})
	if err == nil {
		t.Fatalf("StartRuntimeHost succeeded with invalid request")
	}
	if !errors.Is(err, requestErr) {
		t.Fatalf("StartRuntimeHost error = %v, want %v", err, requestErr)
	}
	if transport.startCalls != 0 {
		t.Fatalf("transport called despite invalid request")
	}
}

func TestDaemonAttachRejectsControlOnlyReadiness(t *testing.T) {
	transport := &memoryDaemonTransport{
		attachJSON: `{"handle_id":"daemon-1","state":"ControlOnly","endpoints":{"control_endpoint":"unix:///tmp/control.sock"}}`,
	}

	_, err := AttachRuntimeHost(context.Background(), transport, RuntimeHostAttachOptions{ControlEndpoint: "unix:///tmp/control.sock"})
	if err == nil {
		t.Fatalf("Attach succeeded with control-only readiness")
	}
	if !IsCode(err, ErrControlOnly) {
		t.Fatalf("error = %v, want ControlOnly", err)
	}
}

func TestRuntimeHostDiscoverPreservesAdvertisedInvocationEndpoint(t *testing.T) {
	transport := &memoryDaemonTransport{
		discoverJSON: `{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/custom-daemon.sock"}`,
	}

	endpoints, err := DiscoverRuntimeHost(context.Background(), transport, RuntimeHostDiscoverOptions{ControlPath: "/tmp/control.sock"})
	if err != nil {
		t.Fatalf("DiscoverRuntimeHost: %v", err)
	}
	if endpoints.InvocationEndpoint != "unix:///tmp/custom-daemon.sock" {
		t.Fatalf("did not preserve advertised invocation endpoint: %#v", endpoints)
	}
}

func TestDaemonHandleOpenRuntimeRequiresReadyState(t *testing.T) {
	transport := &memoryDaemonTransport{startJSON: readyDaemonStatus()}
	handle, err := StartRuntimeHost(context.Background(), transport, testRuntimeHostStartRequest{
		payload: map[string]any{"mode": "test-runtime"},
	})
	if err != nil {
		t.Fatalf("StartRuntimeHost: %v", err)
	}

	client, err := handle.OpenRuntime(context.Background(), ConnectOptions{MaxMessageBytes: 4096})
	if err != nil {
		t.Fatalf("OpenRuntime: %v", err)
	}
	if client == nil || transport.openCalls != 1 || transport.seenOptions["max_message_bytes"] != float64(4096) {
		t.Fatalf("runtime not opened correctly: client=%#v calls=%d options=%#v", client, transport.openCalls, transport.seenOptions)
	}

	handle.status.State = RuntimeControlReady
	if _, err := handle.OpenRuntime(context.Background(), ConnectOptions{}); err == nil {
		t.Fatalf("OpenRuntime succeeded from ControlReady")
	}
}

func TestConnectLocalDiscoversAttachesOpensAndDetaches(t *testing.T) {
	transport := &memoryDaemonTransport{
		discoverJSON: `{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/discovered-daemon.sock"}`,
		attachJSON:   readyDaemonStatus(),
	}

	client, err := ConnectLocalRuntimeHost(context.Background(), transport, ConnectOptions{ControlPath: "/tmp/control.sock", MaxMessageBytes: 4096})
	if err != nil {
		t.Fatalf("ConnectLocal: %v", err)
	}
	if client == nil {
		t.Fatal("runtime client is nil")
	}
	if transport.openCalls != 1 || transport.detachCalls != 1 {
		t.Fatalf("open/detach calls = %d/%d", transport.openCalls, transport.detachCalls)
	}
	if transport.seenOptions["endpoint"] != "unix:///tmp/discovered-daemon.sock" {
		t.Fatalf("open runtime did not use discovered endpoint: %#v", transport.seenOptions)
	}
	if transport.seenOptions["max_message_bytes"] != float64(4096) {
		t.Fatalf("connect options not preserved: %#v", transport.seenOptions)
	}
}

func TestConnectLocalRejectsControlOnlyAttach(t *testing.T) {
	transport := &memoryDaemonTransport{
		discoverJSON: `{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/daemon.sock"}`,
		attachJSON:   `{"handle_id":"daemon-1","state":"ControlOnly","endpoints":{"control_endpoint":"unix:///tmp/control.sock"}}`,
	}

	_, err := ConnectLocalRuntimeHost(context.Background(), transport, ConnectOptions{})
	if err == nil {
		t.Fatal("expected control-only rejection")
	}
	if !IsCode(err, ErrControlOnly) {
		t.Fatalf("err = %v, want ErrControlOnly", err)
	}
	if transport.openCalls != 0 || transport.detachCalls != 0 {
		t.Fatalf("unexpected open/detach calls = %d/%d", transport.openCalls, transport.detachCalls)
	}
}

func TestConnectLocalDetachesAfterOpenRuntimeFailure(t *testing.T) {
	openErr := errors.New("open failed")
	transport := &memoryDaemonTransport{
		discoverJSON: `{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/daemon.sock"}`,
		attachJSON:   readyDaemonStatus(),
		openErr:      openErr,
	}

	_, err := ConnectLocalRuntimeHost(context.Background(), transport, ConnectOptions{})
	if err == nil {
		t.Fatal("expected open runtime failure")
	}
	if !errors.Is(err, openErr) {
		t.Fatalf("err = %v, want wrapped openErr", err)
	}
	if transport.openCalls != 1 || transport.detachCalls != 1 {
		t.Fatalf("open/detach calls = %d/%d", transport.openCalls, transport.detachCalls)
	}
}

func TestConnectLocalAllowsExplicitEndpointOverride(t *testing.T) {
	transport := &memoryDaemonTransport{
		discoverJSON: `{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/discovered-daemon.sock"}`,
		attachJSON:   readyDaemonStatus(),
	}

	_, err := ConnectLocalRuntimeHost(context.Background(), transport, ConnectOptions{Endpoint: "unix:///tmp/explicit-daemon.sock"})
	if err != nil {
		t.Fatalf("ConnectLocal: %v", err)
	}
	if transport.seenOptions["endpoint"] != "unix:///tmp/explicit-daemon.sock" {
		t.Fatalf("explicit endpoint not forwarded: %#v", transport.seenOptions)
	}
}

func TestDaemonStopIsIdempotentAndDetachDoesNotStop(t *testing.T) {
	transport := &memoryDaemonTransport{
		startJSON: readyDaemonStatus(),
		stopJSON:  `{"handle_id":"daemon-1","state":"Stopped","mode":"authority"}`,
	}
	handle, err := StartRuntimeHost(context.Background(), transport, testRuntimeHostStartRequest{
		payload: map[string]any{"mode": "test-runtime"},
	})
	if err != nil {
		t.Fatalf("StartRuntimeHost: %v", err)
	}

	if err := handle.StopRuntime(context.Background(), RuntimeHostStopOptions{}); err != nil {
		t.Fatalf("Stop: %v", err)
	}
	if err := handle.StopRuntime(context.Background(), RuntimeHostStopOptions{}); err != nil {
		t.Fatalf("Stop second: %v", err)
	}
	if transport.stopCalls != 1 {
		t.Fatalf("stop calls = %d, want 1", transport.stopCalls)
	}
	if err := handle.Detach(context.Background()); err != nil {
		t.Fatalf("Detach: %v", err)
	}
	if transport.detachCalls != 1 || transport.stopCalls != 1 {
		t.Fatalf("detach changed stop behavior: detach=%d stop=%d", transport.detachCalls, transport.stopCalls)
	}
	if _, err := handle.Status(context.Background()); err == nil {
		t.Fatalf("Status succeeded after detach")
	}
}
