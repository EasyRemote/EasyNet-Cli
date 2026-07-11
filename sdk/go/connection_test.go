package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"testing"
)

type memoryRuntimeConnector struct {
	seenOptions  map[string]any
	closed       bool
	resolveErr   error
	handshakeErr error
}

func (m *memoryRuntimeConnector) Resolve(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if m.resolveErr != nil {
		return nil, m.resolveErr
	}
	if err := json.Unmarshal(optionsJSON, &m.seenOptions); err != nil {
		return nil, err
	}
	return []byte(`{"endpoint": "unix:///tmp/easynet-daemon.sock", "control_path": "/tmp/control.sock", "protocol_version": "v4", "abi_version": 5}`), nil
}

func (m *memoryRuntimeConnector) Handshake(ctx context.Context, endpointJSON []byte) (RuntimeTransport, []byte, error) {
	if m.handshakeErr != nil {
		return nil, nil, m.handshakeErr
	}
	return RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			return []byte(`{}`), nil
		},
	}, []byte(`{"ready": true}`), nil
}

func (m *memoryRuntimeConnector) Close(ctx context.Context) error {
	m.closed = true
	return nil
}

func TestRuntimeConnectionConnectsToReadyClient(t *testing.T) {
	connector := &memoryRuntimeConnector{}
	connection, err := NewRuntimeConnection(connector)
	if err != nil {
		t.Fatalf("NewRuntimeConnection: %v", err)
	}

	if err := connection.Connect(context.Background(), ConnectOptions{Endpoint: "unix:///tmp/easynet-daemon.sock", MaxMessageBytes: 4096}); err != nil {
		t.Fatalf("Connect: %v", err)
	}

	if connection.State() != ConnectionReady {
		t.Fatalf("state = %s, want Ready", connection.State())
	}
	if connection.Endpoint().Endpoint != "unix:///tmp/easynet-daemon.sock" {
		t.Fatalf("endpoint = %#v", connection.Endpoint())
	}
	if connector.seenOptions["max_message_bytes"] != float64(4096) {
		t.Fatalf("options not forwarded: %#v", connector.seenOptions)
	}
	client, err := connection.RuntimeClient()
	if err != nil {
		t.Fatalf("RuntimeClient: %v", err)
	}
	if client == nil {
		t.Fatalf("RuntimeClient returned nil")
	}
}

func TestRuntimeConnectionFailureIsTerminalForClient(t *testing.T) {
	down := errors.New("daemon down")
	connection, err := NewRuntimeConnection(&memoryRuntimeConnector{resolveErr: down})
	if err != nil {
		t.Fatalf("NewRuntimeConnection: %v", err)
	}

	err = connection.Connect(context.Background(), ConnectOptions{})
	if err == nil {
		t.Fatalf("Connect succeeded, want failure")
	}
	if connection.State() != ConnectionFailed {
		t.Fatalf("state = %s, want Failed", connection.State())
	}
	if !errors.Is(err, down) {
		t.Fatalf("cause not preserved")
	}
	if _, err := connection.RuntimeClient(); err == nil {
		t.Fatalf("RuntimeClient succeeded from failed connection")
	}
}

func TestRuntimeConnectionCloseIsTerminal(t *testing.T) {
	connector := &memoryRuntimeConnector{}
	connection, err := NewRuntimeConnection(connector)
	if err != nil {
		t.Fatalf("NewRuntimeConnection: %v", err)
	}
	if err := connection.Connect(context.Background(), ConnectOptions{}); err != nil {
		t.Fatalf("Connect: %v", err)
	}

	if err := connection.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if !connector.closed {
		t.Fatalf("connector was not closed")
	}
	if connection.State() != ConnectionClosed {
		t.Fatalf("state = %s, want Closed", connection.State())
	}
	if _, err := connection.RuntimeClient(); err == nil {
		t.Fatalf("RuntimeClient succeeded from closed connection")
	}
	if err := connection.Connect(context.Background(), ConnectOptions{}); err == nil {
		t.Fatalf("Connect succeeded after close")
	}
}
