package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

const defaultControlDiscoveryPath = ".easynet/control.json"

// IpcVersionRange is the daemon control-plane discovery version range.
type IpcVersionRange struct {
	Min int `json:"min"`
	Max int `json:"max"`
}

// ControlDiscovery is the local daemon control.json projection.
//
// It is discovery data only. Invocation wire encoding, signing, routing, and
// Axon protocol semantics stay behind the RuntimeTransport selected by
// RuntimeConnection handshake.
type ControlDiscovery struct {
	SocketPath           string          `json:"socket_path,omitempty"`
	PipeName             string          `json:"pipe_name,omitempty"`
	InvocationEndpoint   string          `json:"invocation_endpoint,omitempty"`
	PID                  int             `json:"pid,omitempty"`
	DaemonVersion        string          `json:"daemon_version,omitempty"`
	SupportedIPCVersions IpcVersionRange `json:"supported_ipc_versions,omitempty"`
	CapabilityFlags      []string        `json:"capability_flags,omitempty"`
	PagesPort            int             `json:"pages_port,omitempty"`
}

// ControlDiscoveryReader supplies daemon discovery facts to runtime connectors.
type ControlDiscoveryReader interface {
	ReadControlDiscovery(ctx context.Context, controlPath string) (ControlDiscovery, error)
}

type fileControlDiscoveryReader struct{}

func (fileControlDiscoveryReader) ReadControlDiscovery(ctx context.Context, controlPath string) (ControlDiscovery, error) {
	if ctx == nil {
		return ControlDiscovery{}, invalidRuntimeClient("context is required")
	}
	select {
	case <-ctx.Done():
		return ControlDiscovery{}, transportRuntimeError("read control discovery cancelled", ctx.Err())
	default:
	}
	path, err := resolveControlDiscoveryPath(controlPath)
	if err != nil {
		return ControlDiscovery{}, err
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		return ControlDiscovery{}, &SDKError{
			Code:      ErrDaemonOffline,
			Stage:     "control_discovery",
			Retry:     RetrySafe,
			Retryable: RetryableForHint(RetrySafe),
			Message:   fmt.Sprintf("control discovery not readable at %s", path),
			Details:   map[string]any{"control_path": path},
			Cause:     err,
		}
	}
	return newControlDiscoveryFromJSON(raw)
}

func resolveControlDiscoveryPath(controlPath string) (string, error) {
	if controlPath != "" {
		return controlPath, nil
	}
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		return "", &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "control_discovery",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "home directory is required to resolve control discovery path",
			Cause:     err,
		}
	}
	return filepath.Join(home, defaultControlDiscoveryPath), nil
}

func newControlDiscoveryFromJSON(raw []byte) (ControlDiscovery, error) {
	var discovery ControlDiscovery
	if err := json.Unmarshal(raw, &discovery); err != nil {
		return ControlDiscovery{}, invalidRuntimePayload(fmt.Sprintf("decode control discovery JSON: %v", err), err)
	}
	if discovery.CapabilityFlags == nil {
		discovery.CapabilityFlags = []string{}
	}
	return discovery, nil
}

// ControlDiscoveryRuntimeConnector resolves RuntimeEndpoint from daemon
// control discovery and delegates handshake to an inner connector.
type ControlDiscoveryRuntimeConnector struct {
	inner       RuntimeConnector
	controlPath string
	reader      ControlDiscoveryReader
	closed      bool
}

func newControlDiscoveryRuntimeConnector(inner RuntimeConnector, controlPath string, reader ControlDiscoveryReader) (*ControlDiscoveryRuntimeConnector, error) {
	if inner == nil {
		return nil, invalidRuntimeClient("inner runtime connector is required")
	}
	if reader == nil {
		reader = fileControlDiscoveryReader{}
	}
	return &ControlDiscoveryRuntimeConnector{
		inner:       inner,
		controlPath: controlPath,
		reader:      reader,
	}, nil
}

func (c *ControlDiscoveryRuntimeConnector) Resolve(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if err := c.requireOpen(ctx); err != nil {
		return nil, err
	}
	options, err := decodeConnectOptionsJSON(optionsJSON)
	if err != nil {
		return nil, err
	}
	controlPath := options.ControlPath
	if controlPath == "" {
		controlPath = c.controlPath
	}
	if options.Endpoint != "" {
		return runtimeEndpointJSON(RuntimeEndpoint{
			Endpoint:    options.Endpoint,
			ControlPath: controlPath,
		})
	}
	discovery, err := c.reader.ReadControlDiscovery(ctx, controlPath)
	if err != nil {
		return nil, err
	}
	if discovery.InvocationEndpoint == "" {
		return nil, &SDKError{
			Code:      ErrControlOnly,
			Stage:     "control_discovery",
			Retry:     RetrySafe,
			Retryable: RetryableForHint(RetrySafe),
			Message:   "control discovery did not advertise invocation_endpoint",
			Details:   map[string]any{"control_path": controlPath},
		}
	}
	return runtimeEndpointJSON(RuntimeEndpoint{
		Endpoint:        discovery.InvocationEndpoint,
		ControlPath:     controlPath,
		ProtocolVersion: "axon.v1.Invocation",
		ABIVersion:      0,
	})
}

func (c *ControlDiscoveryRuntimeConnector) Handshake(ctx context.Context, endpointJSON []byte) (RuntimeTransport, []byte, error) {
	if err := c.requireOpen(ctx); err != nil {
		return nil, nil, err
	}
	return c.inner.Handshake(ctx, endpointJSON)
}

func (c *ControlDiscoveryRuntimeConnector) Close(ctx context.Context) error {
	if c == nil {
		return invalidRuntimeClient("runtime connector is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	if c.closed {
		return nil
	}
	c.closed = true
	return c.inner.Close(ctx)
}

func (c *ControlDiscoveryRuntimeConnector) requireOpen(ctx context.Context) error {
	if c == nil || c.inner == nil || c.reader == nil {
		return invalidRuntimeClient("runtime connector is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	if c.closed {
		return invalidRuntimeClient("runtime connector is closed")
	}
	return nil
}

func decodeConnectOptionsJSON(raw []byte) (ConnectOptions, error) {
	var options ConnectOptions
	if len(raw) == 0 {
		return options, nil
	}
	if err := json.Unmarshal(raw, &options); err != nil {
		return ConnectOptions{}, invalidRuntimePayload(fmt.Sprintf("decode connect options JSON: %v", err), err)
	}
	return options, nil
}

func runtimeEndpointJSON(endpoint RuntimeEndpoint) ([]byte, error) {
	raw, err := json.Marshal(endpoint)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode runtime endpoint JSON: %v", err), err)
	}
	return raw, nil
}
