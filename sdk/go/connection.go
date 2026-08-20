package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
)

// ConnectionState is the Runtime Core client connection state.
type ConnectionState string

const (
	ConnectionIdle         ConnectionState = "Idle"
	ConnectionResolving    ConnectionState = "Resolving"
	ConnectionConnecting   ConnectionState = "Connecting"
	ConnectionReady        ConnectionState = "Ready"
	ConnectionDegraded     ConnectionState = "Degraded"
	ConnectionReconnecting ConnectionState = "Reconnecting"
	ConnectionFailed       ConnectionState = "Failed"
	ConnectionClosed       ConnectionState = "Closed"
)

// ConnectOptions are Runtime Core connection knobs.
type ConnectOptions struct {
	Endpoint        string `json:"endpoint,omitempty"`
	ControlPath     string `json:"control_path,omitempty"`
	DialTimeoutMS   int64  `json:"dial_timeout_ms,omitempty"`
	InvokeTimeoutMS int64  `json:"invoke_timeout_ms,omitempty"`
	MaxMessageBytes int    `json:"max_message_bytes,omitempty"`
	Reconnect       bool   `json:"reconnect,omitempty"`
}

// RuntimeEndpoint is the resolved runtime invocation endpoint projection.
type RuntimeEndpoint struct {
	Endpoint        string `json:"endpoint"`
	ControlPath     string `json:"control_path,omitempty"`
	ProtocolVersion string `json:"protocol_version,omitempty"`
	ABIVersion      uint32 `json:"abi_version,omitempty"`
}

// RuntimeConnector supplies concrete provider connection steps for RuntimeConnection.
type RuntimeConnector interface {
	Resolve(ctx context.Context, optionsJSON []byte) ([]byte, error)
	Handshake(ctx context.Context, endpointJSON []byte) (RuntimeTransport, []byte, error)
	Close(ctx context.Context) error
}

// RuntimeConnectorFunc adapts functions into a RuntimeConnector.
type RuntimeConnectorFunc struct {
	ResolveFunc   func(ctx context.Context, optionsJSON []byte) ([]byte, error)
	HandshakeFunc func(ctx context.Context, endpointJSON []byte) (RuntimeTransport, []byte, error)
	CloseFunc     func(ctx context.Context) error
}

func (f RuntimeConnectorFunc) Resolve(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if f.ResolveFunc == nil {
		return nil, invalidRuntimeClient("runtime resolve connector function is required")
	}
	return f.ResolveFunc(ctx, optionsJSON)
}

func (f RuntimeConnectorFunc) Handshake(ctx context.Context, endpointJSON []byte) (RuntimeTransport, []byte, error) {
	if f.HandshakeFunc == nil {
		return nil, nil, invalidRuntimeClient("runtime handshake connector function is required")
	}
	return f.HandshakeFunc(ctx, endpointJSON)
}

func (f RuntimeConnectorFunc) Close(ctx context.Context) error {
	if f.CloseFunc == nil {
		return nil
	}
	return f.CloseFunc(ctx)
}

// RuntimeConnection owns client connection state and gates RuntimeClient creation.
type RuntimeConnection struct {
	connector RuntimeConnector
	state     ConnectionState
	endpoint  RuntimeEndpoint
	transport RuntimeTransport
	handshake map[string]any
	lastError error
}

// NewRuntimeConnection creates an idle Runtime Core connection object.
func NewRuntimeConnection(connector RuntimeConnector) (*RuntimeConnection, error) {
	if connector == nil {
		return nil, invalidRuntimeClient("runtime connector is required")
	}
	return &RuntimeConnection{
		connector: connector,
		state:     ConnectionIdle,
		handshake: map[string]any{},
	}, nil
}

// Connect resolves and handshakes the runtime invocation transport.
func (c *RuntimeConnection) Connect(ctx context.Context, opts ConnectOptions) error {
	if c == nil || c.connector == nil {
		return invalidRuntimeClient("runtime connection is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	if c.state == ConnectionClosed {
		return invalidRuntimeClient("runtime connection is closed")
	}
	optionsJSON, err := json.Marshal(opts)
	if err != nil {
		c.fail(invalidRuntimePayload(fmt.Sprintf("encode connect options: %v", err), err))
		return c.lastError
	}
	attempts := 1
	if opts.Reconnect {
		attempts = 2
	}
	for attempt := 0; attempt < attempts; attempt++ {
		if err := c.transition(ConnectionResolving); err != nil {
			return err
		}
		if err := c.connectAttempt(ctx, optionsJSON); err == nil {
			return nil
		} else {
			c.lastError = err
		}
		if attempt+1 == attempts {
			c.fail(c.lastError)
			return c.lastError
		}
		if err := c.transition(ConnectionDegraded); err != nil {
			return err
		}
		if err := c.transition(ConnectionReconnecting); err != nil {
			return err
		}
	}
	return c.lastError
}

func (c *RuntimeConnection) connectAttempt(ctx context.Context, optionsJSON []byte) error {
	rawEndpoint, err := c.connector.Resolve(ctx, optionsJSON)
	if err != nil {
		return wrapRuntimeConnectError("resolve runtime endpoint failed", err)
	}
	endpoint, err := NewRuntimeEndpointFromJSON(rawEndpoint)
	if err != nil {
		return err
	}
	c.endpoint = endpoint
	if err := c.transition(ConnectionConnecting); err != nil {
		return err
	}
	transport, rawHandshake, err := c.connector.Handshake(ctx, rawEndpoint)
	if err != nil {
		return wrapRuntimeConnectError("runtime handshake failed", err)
	}
	if transport == nil {
		return invalidRuntimeClient("runtime transport is required after handshake")
	}
	handshake, err := decodeHandshakeFacts(rawHandshake)
	if err != nil {
		return err
	}
	c.transport = transport
	c.handshake = handshake
	c.lastError = nil
	return c.transition(ConnectionReady)
}

// RuntimeClient returns a RuntimeClient only when the connection is ready.
func (c *RuntimeConnection) RuntimeClient() (*RuntimeClient, error) {
	if c == nil || c.connector == nil {
		return nil, invalidRuntimeClient("runtime connection is not initialized")
	}
	if c.state != ConnectionReady || c.transport == nil {
		return nil, invalidRuntimeClient("runtime connection is not ready")
	}
	return NewRuntimeClient(c.transport)
}

// Close closes the connection and moves it to the terminal Closed state.
func (c *RuntimeConnection) Close(ctx context.Context) error {
	if c == nil || c.connector == nil {
		return invalidRuntimeClient("runtime connection is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	if c.state == ConnectionClosed {
		return nil
	}
	err := c.connector.Close(ctx)
	c.transport = nil
	if transitionErr := c.transition(ConnectionClosed); transitionErr != nil {
		return transitionErr
	}
	if err != nil {
		c.lastError = wrapRuntimeConnectError("runtime close failed", err)
		return c.lastError
	}
	c.lastError = nil
	return nil
}

func (c *RuntimeConnection) State() ConnectionState {
	if c == nil {
		return ConnectionFailed
	}
	return c.state
}

func (c *RuntimeConnection) Endpoint() RuntimeEndpoint {
	if c == nil {
		return RuntimeEndpoint{}
	}
	return c.endpoint
}

func (c *RuntimeConnection) HandshakeFacts() map[string]any {
	if c == nil {
		return map[string]any{}
	}
	return copyMap(c.handshake)
}

func (c *RuntimeConnection) LastError() error {
	if c == nil {
		return nil
	}
	return c.lastError
}

func (c *RuntimeConnection) fail(err error) {
	c.transport = nil
	c.lastError = err
	if c.state != ConnectionClosed {
		if transitionErr := c.transition(ConnectionFailed); transitionErr != nil {
			c.lastError = transitionErr
		}
	}
}

func (c *RuntimeConnection) transition(next ConnectionState) error {
	allowed := map[ConnectionState]map[ConnectionState]bool{
		ConnectionIdle:         {ConnectionResolving: true, ConnectionClosed: true},
		ConnectionResolving:    {ConnectionConnecting: true, ConnectionDegraded: true, ConnectionFailed: true, ConnectionClosed: true},
		ConnectionConnecting:   {ConnectionReady: true, ConnectionDegraded: true, ConnectionFailed: true, ConnectionClosed: true},
		ConnectionReady:        {ConnectionResolving: true, ConnectionClosed: true},
		ConnectionDegraded:     {ConnectionReconnecting: true, ConnectionFailed: true, ConnectionClosed: true},
		ConnectionReconnecting: {ConnectionResolving: true, ConnectionFailed: true, ConnectionClosed: true},
		ConnectionFailed:       {ConnectionResolving: true, ConnectionClosed: true},
		ConnectionClosed:       {ConnectionClosed: true},
	}
	if !allowed[c.state][next] {
		return invalidRuntimeClient(fmt.Sprintf("runtime connection cannot transition from %s to %s", c.state, next))
	}
	c.state = next
	return nil
}

// NewRuntimeEndpointFromJSON decodes runtime endpoint discovery JSON.
func NewRuntimeEndpointFromJSON(raw []byte) (RuntimeEndpoint, error) {
	var endpoint RuntimeEndpoint
	if err := json.Unmarshal(raw, &endpoint); err != nil {
		return RuntimeEndpoint{}, invalidRuntimePayload(fmt.Sprintf("decode runtime endpoint JSON: %v", err), err)
	}
	if endpoint.Endpoint == "" {
		return RuntimeEndpoint{}, invalidRuntimePayload("endpoint is required", nil)
	}
	return endpoint, nil
}

func decodeHandshakeFacts(raw []byte) (map[string]any, error) {
	if len(raw) == 0 {
		return map[string]any{}, nil
	}
	var facts map[string]any
	if err := json.Unmarshal(raw, &facts); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode runtime handshake JSON: %v", err), err)
	}
	if facts == nil {
		return map[string]any{}, nil
	}
	return facts, nil
}

func wrapRuntimeConnectError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
