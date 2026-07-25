package easynet

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
)

const defaultControlDiscoveryPath = ".easynet/control.json"

// IpcVersionRange is the runtime-host control-plane discovery version range.
type IpcVersionRange struct {
	Min int `json:"min"`
	Max int `json:"max"`
}

// controlDiscovery is the local runtime-host control.json projection.
//
// It is discovery data only. Invocation wire encoding, signing, routing, and
// Axon protocol semantics stay behind the RuntimeTransport selected by
// RuntimeConnection handshake.
type controlDiscovery struct {
	socketPath           string
	pipeName             string
	invocationEndpoint   string
	pid                  int
	runtimeHostVersion   string
	supportedIPCVersions IpcVersionRange
	capabilityFlags      []string
	pagesPort            int
}

type controlDiscoveryJSON struct {
	SocketPath           *string                               `json:"socket_path,omitempty"`
	PipeName             *string                               `json:"pipe_name,omitempty"`
	InvocationEndpoint   *string                               `json:"invocation_endpoint,omitempty"`
	RuntimeHostIdentity  *controlRuntimeHostIdentityProjection `json:"daemon_identity,omitempty"`
	PID                  *int                                  `json:"pid,omitempty"`
	RuntimeHostVersion   *string                               `json:"daemon_version,omitempty"`
	SupportedIPCVersions *IpcVersionRange                      `json:"supported_ipc_versions,omitempty"`
	CapabilityFlags      *[]string                             `json:"capability_flags,omitempty"`
	PagesPort            *int                                  `json:"pages_port,omitempty"`
}

// controlRuntimeHostIdentityProjection isolates current provider wire keys
// from the SDK domain model. The provider discovery file still spells this
// object as daemon_identity/node_id; SDK internals treat it as a runtime-host
// identity with a runtime instance id.
type controlRuntimeHostIdentityProjection struct {
	Mode              string  `json:"mode"`
	Realm             string  `json:"realm"`
	RuntimeInstanceID *string `json:"node_id,omitempty"`
}

func (d *controlDiscovery) UnmarshalJSON(raw []byte) error {
	var wire controlDiscoveryJSON
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&wire); err != nil {
		return err
	}
	if decoder.Decode(&struct{}{}) != io.EOF {
		return fmt.Errorf("control discovery JSON contains trailing data")
	}
	if wire.PID == nil || *wire.PID <= 0 {
		return fmt.Errorf("control discovery pid is required")
	}
	if wire.RuntimeHostVersion == nil || *wire.RuntimeHostVersion == "" {
		return fmt.Errorf("control discovery daemon_version is required")
	}
	if wire.SupportedIPCVersions == nil ||
		wire.SupportedIPCVersions.Min <= 0 ||
		wire.SupportedIPCVersions.Max <= 0 ||
		wire.SupportedIPCVersions.Min > wire.SupportedIPCVersions.Max {
		return fmt.Errorf("control discovery supported_ipc_versions is required")
	}
	if wire.CapabilityFlags == nil {
		return fmt.Errorf("control discovery capability_flags is required")
	}
	for _, flag := range *wire.CapabilityFlags {
		if flag == "" {
			return fmt.Errorf("control discovery capability_flags must contain non-empty strings")
		}
	}
	if wire.PagesPort != nil && (*wire.PagesPort <= 0 || *wire.PagesPort > 65535) {
		return fmt.Errorf("control discovery pages_port must be a positive TCP port")
	}
	d.socketPath = stringPointerValue(wire.SocketPath)
	d.pipeName = stringPointerValue(wire.PipeName)
	d.invocationEndpoint = stringPointerValue(wire.InvocationEndpoint)
	d.pid = *wire.PID
	d.runtimeHostVersion = *wire.RuntimeHostVersion
	d.supportedIPCVersions = *wire.SupportedIPCVersions
	d.capabilityFlags = append([]string(nil), (*wire.CapabilityFlags)...)
	d.pagesPort = intPointerValue(wire.PagesPort)
	return nil
}

// controlDiscoveryReader supplies runtime-host discovery facts to runtime connectors.
type controlDiscoveryReader interface {
	readControlDiscovery(ctx context.Context, controlPath string) (controlDiscovery, error)
}

type fileControlDiscoveryReader struct{}

func (fileControlDiscoveryReader) readControlDiscovery(ctx context.Context, controlPath string) (controlDiscovery, error) {
	if ctx == nil {
		return controlDiscovery{}, invalidRuntimeClient("context is required")
	}
	select {
	case <-ctx.Done():
		return controlDiscovery{}, transportRuntimeError("read control discovery cancelled", ctx.Err())
	default:
	}
	path, err := resolveControlDiscoveryPath(controlPath)
	if err != nil {
		return controlDiscovery{}, err
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		return controlDiscovery{}, &SDKError{
			Code:      ErrRuntimeOffline,
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

func newControlDiscoveryFromJSON(raw []byte) (controlDiscovery, error) {
	var discovery controlDiscovery
	if err := json.Unmarshal(raw, &discovery); err != nil {
		return controlDiscovery{}, invalidRuntimePayload(fmt.Sprintf("decode control discovery JSON: %v", err), err)
	}
	return discovery, nil
}

func stringPointerValue(value *string) string {
	if value == nil {
		return ""
	}
	return *value
}

func intPointerValue(value *int) int {
	if value == nil {
		return 0
	}
	return *value
}

// controlDiscoveryRuntimeConnector resolves RuntimeEndpoint from runtime-host
// control discovery and delegates handshake to an inner connector.
type controlDiscoveryRuntimeConnector struct {
	inner       RuntimeConnector
	controlPath string
	reader      controlDiscoveryReader
	closed      bool
}

func newControlDiscoveryRuntimeConnector(inner RuntimeConnector, controlPath string, reader controlDiscoveryReader) (*controlDiscoveryRuntimeConnector, error) {
	if inner == nil {
		return nil, invalidRuntimeClient("inner runtime connector is required")
	}
	if reader == nil {
		reader = fileControlDiscoveryReader{}
	}
	return &controlDiscoveryRuntimeConnector{
		inner:       inner,
		controlPath: controlPath,
		reader:      reader,
	}, nil
}

func (c *controlDiscoveryRuntimeConnector) Resolve(ctx context.Context, optionsJSON []byte) ([]byte, error) {
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
	discovery, err := c.reader.readControlDiscovery(ctx, controlPath)
	if err != nil {
		return nil, err
	}
	if discovery.invocationEndpoint == "" {
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
		Endpoint:        discovery.invocationEndpoint,
		ControlPath:     controlPath,
		ProtocolVersion: "axon.v1.Invocation",
		ABIVersion:      0,
	})
}

func (c *controlDiscoveryRuntimeConnector) Handshake(ctx context.Context, endpointJSON []byte) (RuntimeTransport, []byte, error) {
	if err := c.requireOpen(ctx); err != nil {
		return nil, nil, err
	}
	return c.inner.Handshake(ctx, endpointJSON)
}

func (c *controlDiscoveryRuntimeConnector) Close(ctx context.Context) error {
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

func (c *controlDiscoveryRuntimeConnector) requireOpen(ctx context.Context) error {
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
