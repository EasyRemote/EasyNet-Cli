package easynet

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

type memoryControlDiscoveryReader struct {
	discovery ControlDiscovery
	err       error
	calls     []string
}

func (r *memoryControlDiscoveryReader) ReadControlDiscovery(ctx context.Context, controlPath string) (ControlDiscovery, error) {
	r.calls = append(r.calls, controlPath)
	if r.err != nil {
		return ControlDiscovery{}, r.err
	}
	return r.discovery, nil
}

func TestControlDiscoveryRuntimeConnectorUsesExplicitEndpointWithoutReadingDiscovery(t *testing.T) {
	inner := &memoryRuntimeConnector{}
	reader := &memoryControlDiscoveryReader{
		discovery: ControlDiscovery{InvocationEndpoint: "unix:///tmp/discovered.sock"},
	}
	connector, err := NewControlDiscoveryRuntimeConnector(inner, "/tmp/default-control.json", reader)
	if err != nil {
		t.Fatalf("NewControlDiscoveryRuntimeConnector: %v", err)
	}

	raw, err := connector.Resolve(context.Background(), []byte(`{
		"endpoint":"unix:///tmp/explicit.sock",
		"control_path":"/tmp/override-control.json"
	}`))
	if err != nil {
		t.Fatalf("Resolve: %v", err)
	}
	endpoint, err := NewRuntimeEndpointFromJSON(raw)
	if err != nil {
		t.Fatalf("NewRuntimeEndpointFromJSON: %v", err)
	}
	if endpoint.Endpoint != "unix:///tmp/explicit.sock" ||
		endpoint.ControlPath != "/tmp/override-control.json" {
		t.Fatalf("endpoint = %#v", endpoint)
	}
	if len(reader.calls) != 0 {
		t.Fatalf("explicit endpoint should not read discovery: %#v", reader.calls)
	}
}

func TestControlDiscoveryRuntimeConnectorReadsInvocationEndpoint(t *testing.T) {
	inner := &memoryRuntimeConnector{}
	reader := &memoryControlDiscoveryReader{
		discovery: ControlDiscovery{
			SocketPath:         "/tmp/control.sock",
			InvocationEndpoint: "unix:///tmp/discovered-daemon.sock",
			DaemonVersion:      "0.91.30",
			CapabilityFlags:    []string{"invocation", "stream"},
		},
	}
	connector, err := NewControlDiscoveryRuntimeConnector(inner, "/tmp/default-control.json", reader)
	if err != nil {
		t.Fatalf("NewControlDiscoveryRuntimeConnector: %v", err)
	}

	connection, err := NewRuntimeConnection(connector)
	if err != nil {
		t.Fatalf("NewRuntimeConnection: %v", err)
	}
	if err := connection.Connect(context.Background(), ConnectOptions{MaxMessageBytes: 8192}); err != nil {
		t.Fatalf("Connect: %v", err)
	}

	if connection.State() != ConnectionReady {
		t.Fatalf("state = %s", connection.State())
	}
	if connection.Endpoint().Endpoint != "unix:///tmp/discovered-daemon.sock" ||
		connection.Endpoint().ControlPath != "/tmp/default-control.json" {
		t.Fatalf("endpoint = %#v", connection.Endpoint())
	}
	if len(reader.calls) != 1 || reader.calls[0] != "/tmp/default-control.json" {
		t.Fatalf("reader calls = %#v", reader.calls)
	}
	if !inner.closed {
		if err := connection.Close(context.Background()); err != nil {
			t.Fatalf("Close: %v", err)
		}
	}
	if !inner.closed {
		t.Fatalf("inner connector was not closed")
	}
}

func TestControlDiscoveryRuntimeConnectorRejectsControlOnlyDiscovery(t *testing.T) {
	connector, err := NewControlDiscoveryRuntimeConnector(
		&memoryRuntimeConnector{},
		"/tmp/control-only.json",
		&memoryControlDiscoveryReader{discovery: ControlDiscovery{SocketPath: "/tmp/control.sock"}},
	)
	if err != nil {
		t.Fatalf("NewControlDiscoveryRuntimeConnector: %v", err)
	}

	_, err = connector.Resolve(context.Background(), nil)
	if !IsCode(err, ErrControlOnly) {
		t.Fatalf("Resolve error = %v, want %s", err, ErrControlOnly)
	}
}

func TestFileControlDiscoveryReaderReadsControlJSON(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "control.json")
	raw := []byte(`{
		"socket_path":"/tmp/control.sock",
		"invocation_endpoint":"unix:///tmp/daemon.sock",
		"pid":42,
		"daemon_version":"0.91.30",
		"supported_ipc_versions":{"min":1,"max":1},
		"capability_flags":["invocation","stream"],
		"pages_port":8080
	}`)
	if err := os.WriteFile(path, raw, 0o600); err != nil {
		t.Fatalf("WriteFile: %v", err)
	}

	discovery, err := FileControlDiscoveryReader{}.ReadControlDiscovery(context.Background(), path)
	if err != nil {
		t.Fatalf("ReadControlDiscovery: %v", err)
	}
	if discovery.InvocationEndpoint != "unix:///tmp/daemon.sock" ||
		discovery.PID != 42 ||
		len(discovery.CapabilityFlags) != 2 {
		t.Fatalf("discovery = %#v", discovery)
	}
}

func TestControlDiscoveryRuntimeConnectorPassesResolvedEndpointToInnerHandshake(t *testing.T) {
	seen := map[string]any{}
	inner := RuntimeConnectorFunc{
		ResolveFunc: func(ctx context.Context, optionsJSON []byte) ([]byte, error) {
			t.Fatalf("inner Resolve must not be called")
			return nil, nil
		},
		HandshakeFunc: func(ctx context.Context, endpointJSON []byte) (RuntimeTransport, []byte, error) {
			if err := json.Unmarshal(endpointJSON, &seen); err != nil {
				t.Fatalf("decode endpoint: %v", err)
			}
			return RuntimeTransportFunc{
				InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
					return []byte(`{}`), nil
				},
			}, []byte(`{"ready":true}`), nil
		},
	}
	connector, err := NewControlDiscoveryRuntimeConnector(
		inner,
		"",
		&memoryControlDiscoveryReader{
			discovery: ControlDiscovery{InvocationEndpoint: "unix:///tmp/daemon.sock"},
		},
	)
	if err != nil {
		t.Fatalf("NewControlDiscoveryRuntimeConnector: %v", err)
	}
	connection, err := NewRuntimeConnection(connector)
	if err != nil {
		t.Fatalf("NewRuntimeConnection: %v", err)
	}

	if err := connection.Connect(context.Background(), ConnectOptions{ControlPath: "/tmp/control.json"}); err != nil {
		t.Fatalf("Connect: %v", err)
	}
	if seen["endpoint"] != "unix:///tmp/daemon.sock" ||
		seen["control_path"] != "/tmp/control.json" ||
		seen["protocol_version"] != "axon.v1.Invocation" {
		t.Fatalf("inner handshake endpoint = %#v", seen)
	}
}
