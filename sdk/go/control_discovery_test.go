package easynet

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type controlDiscoveryReaderFunc func(ctx context.Context, controlPath string) (controlDiscovery, error)

func (f controlDiscoveryReaderFunc) readControlDiscovery(ctx context.Context, controlPath string) (controlDiscovery, error) {
	if f == nil {
		return controlDiscovery{}, invalidRuntimeClient("control discovery reader function is required")
	}
	return f(ctx, controlPath)
}

type memoryControlDiscoveryReader struct {
	discovery controlDiscovery
	err       error
	calls     []string
}

func (r *memoryControlDiscoveryReader) readControlDiscovery(ctx context.Context, controlPath string) (controlDiscovery, error) {
	r.calls = append(r.calls, controlPath)
	if r.err != nil {
		return controlDiscovery{}, r.err
	}
	return r.discovery, nil
}

func TestControlDiscoveryRuntimeConnectorUsesExplicitEndpointWithoutReadingDiscovery(t *testing.T) {
	inner := &memoryRuntimeConnector{}
	reader := &memoryControlDiscoveryReader{
		discovery: controlDiscovery{invocationEndpoint: "unix:///tmp/discovered.sock"},
	}
	connector, err := newControlDiscoveryRuntimeConnector(inner, "/tmp/default-control.json", reader)
	if err != nil {
		t.Fatalf("newControlDiscoveryRuntimeConnector: %v", err)
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
		discovery: controlDiscovery{
			socketPath:         "/tmp/control.sock",
			invocationEndpoint: "unix:///tmp/discovered-runtime.sock",
			runtimeHostVersion: "0.91.30",
			capabilityFlags:    []string{"invocation", "stream"},
		},
	}
	connector, err := newControlDiscoveryRuntimeConnector(inner, "/tmp/default-control.json", reader)
	if err != nil {
		t.Fatalf("newControlDiscoveryRuntimeConnector: %v", err)
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
	if connection.Endpoint().Endpoint != "unix:///tmp/discovered-runtime.sock" ||
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
	connector, err := newControlDiscoveryRuntimeConnector(
		&memoryRuntimeConnector{},
		"/tmp/control-only.json",
		&memoryControlDiscoveryReader{discovery: controlDiscovery{socketPath: "/tmp/control.sock"}},
	)
	if err != nil {
		t.Fatalf("newControlDiscoveryRuntimeConnector: %v", err)
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
		"daemon_identity":{"mode":"device","realm":"localhost","node_id":"node-1"},
		"pid":42,
		"daemon_version":"0.91.30",
		"supported_ipc_versions":{"min":1,"max":1},
		"capability_flags":["invocation","stream"],
		"pages_port":8080
	}`)
	if err := os.WriteFile(path, raw, 0o600); err != nil {
		t.Fatalf("WriteFile: %v", err)
	}

	discovery, err := fileControlDiscoveryReader{}.readControlDiscovery(context.Background(), path)
	if err != nil {
		t.Fatalf("readControlDiscovery: %v", err)
	}
	if discovery.invocationEndpoint != "unix:///tmp/daemon.sock" ||
		discovery.pid != 42 ||
		len(discovery.capabilityFlags) != 2 {
		t.Fatalf("discovery = %#v", discovery)
	}
	if discovery.runtimeHostIdentity == nil ||
		discovery.runtimeHostIdentity.Realm != "localhost" ||
		discovery.runtimeHostIdentity.RuntimeInstanceID == nil ||
		*discovery.runtimeHostIdentity.RuntimeInstanceID != "node-1" {
		t.Fatalf("runtime host identity = %#v", discovery.runtimeHostIdentity)
	}
}

func TestFileControlDiscoveryReaderIgnoresProviderPagesExtension(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "control.json")
	raw := []byte(`{
		"socket_path":"/tmp/control.sock",
		"invocation_endpoint":"unix:///tmp/daemon.sock",
		"pid":42,
		"daemon_version":"0.91.30",
		"supported_ipc_versions":{"min":1,"max":1},
		"capability_flags":["invocation","stream"],
		"pages_port":0
	}`)
	if err := os.WriteFile(path, raw, 0o600); err != nil {
		t.Fatalf("WriteFile: %v", err)
	}

	discovery, err := fileControlDiscoveryReader{}.readControlDiscovery(context.Background(), path)
	if err != nil {
		t.Fatalf("readControlDiscovery: %v", err)
	}
	if discovery.invocationEndpoint != "unix:///tmp/daemon.sock" {
		t.Fatalf("discovery = %#v", discovery)
	}
}

func TestFileControlDiscoveryReaderRejectsLooseControlJSON(t *testing.T) {
	tests := []struct {
		name string
		raw  string
	}{
		{
			name: "unknown field",
			raw: `{
				"socket_path":"/tmp/control.sock",
				"invocation_endpoint":"unix:///tmp/daemon.sock",
				"pid":42,
				"daemon_version":"0.91.30",
				"supported_ipc_versions":{"min":1,"max":1},
				"capability_flags":["invocation"],
				"retired_attach_hint":true
			}`,
		},
		{
			name: "missing capability flags",
			raw: `{
				"socket_path":"/tmp/control.sock",
				"invocation_endpoint":"unix:///tmp/daemon.sock",
				"pid":42,
				"daemon_version":"0.91.30",
				"supported_ipc_versions":{"min":1,"max":1}
			}`,
		},
		{
			name: "incomplete runtime host identity",
			raw: `{
				"daemon_identity":{"mode":"device","realm":"   "},
				"pid":42,
				"daemon_version":"0.91.30",
				"supported_ipc_versions":{"min":1,"max":1},
				"capability_flags":["invocation"]
			}`,
		},
		{
			name: "unknown runtime host identity field",
			raw: `{
				"daemon_identity":{"mode":"device","realm":"localhost","retired_role":"agent"},
				"pid":42,
				"daemon_version":"0.91.30",
				"supported_ipc_versions":{"min":1,"max":1},
				"capability_flags":["invocation"]
			}`,
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			dir := t.TempDir()
			path := filepath.Join(dir, "control.json")
			if err := os.WriteFile(path, []byte(test.raw), 0o600); err != nil {
				t.Fatalf("WriteFile: %v", err)
			}

			_, err := fileControlDiscoveryReader{}.readControlDiscovery(context.Background(), path)
			if !IsCode(err, ErrInvalidArgument) {
				t.Fatalf("readControlDiscovery error = %v, want INVALID_ARGUMENT", err)
			}
		})
	}
}

func TestFileControlDiscoveryReaderNamesRuntimeHostVersionWhenRawFieldMissing(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "control.json")
	raw := []byte(`{
		"socket_path":"/tmp/control.sock",
		"invocation_endpoint":"unix:///tmp/daemon.sock",
		"pid":42,
		"supported_ipc_versions":{"min":1,"max":1},
		"capability_flags":["invocation"]
	}`)
	if err := os.WriteFile(path, raw, 0o600); err != nil {
		t.Fatalf("WriteFile: %v", err)
	}

	_, err := fileControlDiscoveryReader{}.readControlDiscovery(context.Background(), path)
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("readControlDiscovery error = %v, want INVALID_ARGUMENT", err)
	}
	message := err.Error()
	if !strings.Contains(message, "runtime-host version field daemon_version") {
		t.Fatalf("missing version diagnostic = %v, want runtime-host semantic plus raw field", err)
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
	connector, err := newControlDiscoveryRuntimeConnector(
		inner,
		"",
		&memoryControlDiscoveryReader{
			discovery: controlDiscovery{invocationEndpoint: "unix:///tmp/daemon.sock"},
		},
	)
	if err != nil {
		t.Fatalf("newControlDiscoveryRuntimeConnector: %v", err)
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
