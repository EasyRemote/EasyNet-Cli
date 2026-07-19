package easynet

import (
	"context"
	"errors"
	"sync"
)

// SdkEnvironmentOptions are process-level SDK defaults.
type SdkEnvironmentOptions struct {
	ExpectedABIVersion uint32                     `json:"expected_abi_version,omitempty"`
	Discover           RuntimeHostDiscoverOptions `json:"discover,omitempty"`
	Connect            ConnectOptions             `json:"connect,omitempty"`
}

// SdkEnvironment is the process-level Go SDK root.
//
// It owns feature discovery, ABI checks, default runtime discovery/connect
// options, and global SDK cleanup. It never owns Invocation tuple data,
// signing material, runtime process ownership, or receipt verification facts.
type SdkEnvironment struct {
	mu        sync.Mutex
	client    *Client
	host      *RuntimeHost
	lifecycle RuntimeLifecycleTransport
	options   SdkEnvironmentOptions
	closed    bool
}

// NewSdkEnvironment creates a process-level SDK root over public SDK
// transports. Product code receives SDK facades; concrete transports remain
// behind this boundary.
func NewSdkEnvironment(discovery DiscoveryTransport, lifecycle RuntimeLifecycleTransport, opts SdkEnvironmentOptions) (*SdkEnvironment, error) {
	if discovery == nil {
		return nil, invalidRuntimeClient("feature discovery transport is required")
	}
	if lifecycle == nil {
		return nil, invalidRuntimeClient("runtime lifecycle transport is required")
	}
	client, err := NewClient(discovery)
	if err != nil {
		return nil, err
	}
	host, err := NewRuntimeHost(lifecycle)
	if err != nil {
		_ = client.Close(context.Background())
		return nil, err
	}
	return &SdkEnvironment{
		client:    client,
		host:      host,
		lifecycle: lifecycle,
		options:   opts,
	}, nil
}

func (e *SdkEnvironment) requireOpen(ctx context.Context) (*Client, *RuntimeHost, error) {
	if e == nil {
		return nil, nil, invalidRuntimeClient("sdk environment is not initialized")
	}
	if ctx == nil {
		return nil, nil, invalidRuntimeClient("context is required")
	}
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.closed {
		return nil, nil, invalidRuntimeClient("sdk environment is closed")
	}
	if e.client == nil || e.host == nil {
		return nil, nil, invalidRuntimeClient("sdk environment is not initialized")
	}
	return e.client, e.host, nil
}

// FeatureDiscovery reads SDK feature facts through the environment root.
func (e *SdkEnvironment) FeatureDiscovery(ctx context.Context) (FeatureSet, error) {
	client, _, err := e.requireOpen(ctx)
	if err != nil {
		return FeatureSet{}, err
	}
	return client.FeatureDiscovery(ctx)
}

// RequireABI checks the environment's configured ABI version.
func (e *SdkEnvironment) RequireABI(ctx context.Context) (FeatureSet, error) {
	client, _, err := e.requireOpen(ctx)
	if err != nil {
		return FeatureSet{}, err
	}
	expected := e.options.ExpectedABIVersion
	if expected == 0 {
		return FeatureSet{}, invalidRuntimeClient("expected ABI version is required")
	}
	return client.RequireABI(ctx, expected)
}

// RuntimeHost returns the explicit runtime lifecycle facade.
func (e *SdkEnvironment) RuntimeHost(ctx context.Context) (*RuntimeHost, error) {
	_, host, err := e.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return host, nil
}

// DiscoverRuntime discovers runtime endpoints using environment defaults plus
// per-call overrides.
func (e *SdkEnvironment) DiscoverRuntime(ctx context.Context, opts RuntimeHostDiscoverOptions) (RuntimeHostEndpoints, error) {
	_, host, err := e.requireOpen(ctx)
	if err != nil {
		return RuntimeHostEndpoints{}, err
	}
	return host.DiscoverRuntime(ctx, mergeDiscoverOptions(e.options.Discover, opts))
}

// ConnectLocal opens a RuntimeClient through the explicit runtime lifecycle
// facade. It does not start the runtime host.
func (e *SdkEnvironment) ConnectLocal(ctx context.Context, opts ConnectOptions) (*RuntimeClient, error) {
	_, host, err := e.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return host.ConnectLocal(ctx, mergeConnectOptions(e.options.Connect, opts))
}

// Defaults returns the immutable process-level defaults configured on the root.
func (e *SdkEnvironment) Defaults() SdkEnvironmentOptions {
	if e == nil {
		return SdkEnvironmentOptions{}
	}
	e.mu.Lock()
	defer e.mu.Unlock()
	return e.options
}

// Close releases SDK-owned resources without stopping the runtime host.
func (e *SdkEnvironment) Close(ctx context.Context) error {
	if e == nil {
		return invalidRuntimeClient("sdk environment is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	e.mu.Lock()
	if e.closed {
		e.mu.Unlock()
		return nil
	}
	client := e.client
	lifecycle := e.lifecycle
	e.client = nil
	e.host = nil
	e.lifecycle = nil
	e.closed = true
	e.mu.Unlock()

	var closeErr error
	if client != nil {
		closeErr = errors.Join(closeErr, client.Close(ctx))
	}
	if closer, ok := lifecycle.(interface{ Close(context.Context) error }); ok {
		closeErr = errors.Join(closeErr, closer.Close(ctx))
	}
	return closeErr
}

func mergeDiscoverOptions(base RuntimeHostDiscoverOptions, override RuntimeHostDiscoverOptions) RuntimeHostDiscoverOptions {
	if override.ControlEndpoint != "" {
		base.ControlEndpoint = override.ControlEndpoint
	}
	if override.ControlPath != "" {
		base.ControlPath = override.ControlPath
	}
	return base
}

func mergeConnectOptions(base ConnectOptions, override ConnectOptions) ConnectOptions {
	if override.Endpoint != "" {
		base.Endpoint = override.Endpoint
	}
	if override.ControlPath != "" {
		base.ControlPath = override.ControlPath
	}
	if override.DialTimeoutMS != 0 {
		base.DialTimeoutMS = override.DialTimeoutMS
	}
	if override.InvokeTimeoutMS != 0 {
		base.InvokeTimeoutMS = override.InvokeTimeoutMS
	}
	if override.MaxMessageBytes != 0 {
		base.MaxMessageBytes = override.MaxMessageBytes
	}
	if override.Reconnect {
		base.Reconnect = true
	}
	return base
}
