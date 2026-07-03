package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

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
	return `{"handle_id":"daemon-1","state":"Running","mode":"hub","pid":42,"endpoints":{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/daemon.sock","public_endpoint":"https://hub.example"}}`
}

func TestDaemonStartReturnsRuntimeReadyHandle(t *testing.T) {
	transport := &memoryDaemonTransport{startJSON: readyDaemonStatus()}

	handle, err := Start(context.Background(), transport, StartConfig{
		Mode:        ModeHub,
		ListenTCP:   "127.0.0.1:9443",
		TLSCertPath: "/tmp/cert.pem",
		TLSKeyPath:  "/tmp/key.pem",
	})
	if err != nil {
		t.Fatalf("Start: %v", err)
	}

	if handle.HandleID() != "daemon-1" || handle.State() != DaemonRunning {
		t.Fatalf("unexpected handle: id=%q state=%s", handle.HandleID(), handle.State())
	}
	if transport.seenStart["listen_tcp"] != "127.0.0.1:9443" {
		t.Fatalf("start config not forwarded: %#v", transport.seenStart)
	}
	if handle.Endpoints().InvocationEndpoint != "unix:///tmp/daemon.sock" {
		t.Fatalf("invocation endpoint not preserved: %#v", handle.Endpoints())
	}
}

func TestDaemonStartRejectsUnsafeModePolicyBeforeTransport(t *testing.T) {
	transport := &memoryDaemonTransport{startJSON: readyDaemonStatus()}

	_, err := Start(context.Background(), transport, StartConfig{Mode: ModeDevice, ListenTCP: "0.0.0.0:9443"})
	if err == nil {
		t.Fatalf("Start succeeded with device public listener")
	}
	if transport.startCalls != 0 {
		t.Fatalf("transport called despite invalid policy")
	}

	_, err = Start(context.Background(), transport, StartConfig{Mode: ModeHub, ListenTCP: "0.0.0.0:9443"})
	if err == nil {
		t.Fatalf("Start succeeded without TLS material")
	}
	if transport.startCalls != 0 {
		t.Fatalf("transport called despite missing TLS")
	}
}

func TestDaemonAttachRejectsControlOnlyReadiness(t *testing.T) {
	transport := &memoryDaemonTransport{
		attachJSON: `{"handle_id":"daemon-1","state":"ControlOnly","endpoints":{"control_endpoint":"unix:///tmp/control.sock"}}`,
	}

	_, err := Attach(context.Background(), transport, AttachOptions{ControlEndpoint: "unix:///tmp/control.sock"})
	if err == nil {
		t.Fatalf("Attach succeeded with control-only readiness")
	}
	if !IsCode(err, ErrControlOnly) {
		t.Fatalf("error = %v, want ControlOnly", err)
	}
}

func TestDaemonDiscoverPreservesAdvertisedInvocationEndpoint(t *testing.T) {
	transport := &memoryDaemonTransport{
		discoverJSON: `{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/custom-daemon.sock"}`,
	}

	endpoints, err := Discover(context.Background(), transport, DiscoverOptions{HomeDir: "/tmp/easynet-home"})
	if err != nil {
		t.Fatalf("Discover: %v", err)
	}
	if endpoints.InvocationEndpoint != "unix:///tmp/custom-daemon.sock" {
		t.Fatalf("did not preserve advertised invocation endpoint: %#v", endpoints)
	}
}

func TestDaemonHandleOpenRuntimeRequiresReadyState(t *testing.T) {
	transport := &memoryDaemonTransport{startJSON: readyDaemonStatus()}
	handle, err := Start(context.Background(), transport, StartConfig{Mode: ModeHub})
	if err != nil {
		t.Fatalf("Start: %v", err)
	}

	client, err := handle.OpenRuntime(context.Background(), ConnectOptions{MaxMessageBytes: 4096})
	if err != nil {
		t.Fatalf("OpenRuntime: %v", err)
	}
	if client == nil || transport.openCalls != 1 || transport.seenOptions["max_message_bytes"] != float64(4096) {
		t.Fatalf("runtime not opened correctly: client=%#v calls=%d options=%#v", client, transport.openCalls, transport.seenOptions)
	}

	handle.status.State = DaemonControlReady
	if _, err := handle.OpenRuntime(context.Background(), ConnectOptions{}); err == nil {
		t.Fatalf("OpenRuntime succeeded from ControlReady")
	}
}

func TestDaemonStopIsIdempotentAndDetachDoesNotStop(t *testing.T) {
	transport := &memoryDaemonTransport{
		startJSON: readyDaemonStatus(),
		stopJSON:  `{"handle_id":"daemon-1","state":"Stopped","mode":"hub"}`,
	}
	handle, err := Start(context.Background(), transport, StartConfig{Mode: ModeHub})
	if err != nil {
		t.Fatalf("Start: %v", err)
	}

	if err := handle.Stop(context.Background(), StopOptions{}); err != nil {
		t.Fatalf("Stop: %v", err)
	}
	if err := handle.Stop(context.Background(), StopOptions{}); err != nil {
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
