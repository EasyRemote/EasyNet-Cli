// Package easynet binds the canonical runtime model to easynet-daemon.
package easynet

import (
	"context"

	runtimesdk "easynet.run/cli/sdk/go"
	"easynet.run/cli/sdk/go/provider/easynet/contract"
)

type Mode = contract.Mode

const (
	ModeDevice = contract.ModeDevice
	ModeHub    = contract.ModeHub
	ModeBoth   = contract.ModeBoth
)

type StartConfig = contract.StartConfig
type AttachOptions = contract.AttachOptions
type DiscoverOptions = contract.DiscoverOptions
type StopOptions = contract.StopOptions

// Lifecycle is the EasyNet provider facade over one canonical RuntimeHost.
// It owns no lifecycle state and performs no fallback provider selection.
type Lifecycle struct {
	host *runtimesdk.RuntimeHost
}

// NewLifecycle binds an explicit EasyNet lifecycle transport to the canonical host.
func NewLifecycle(transport runtimesdk.RuntimeLifecycleTransport) (*Lifecycle, error) {
	host, err := runtimesdk.NewRuntimeHost(transport)
	if err != nil {
		return nil, err
	}
	return &Lifecycle{host: host}, nil
}

func (l *Lifecycle) Discover(ctx context.Context, opts DiscoverOptions) (runtimesdk.RuntimeHostEndpoints, error) {
	return l.host.DiscoverRuntime(ctx, opts)
}

func (l *Lifecycle) Start(ctx context.Context, cfg StartConfig) (*runtimesdk.RuntimeHandle, error) {
	return l.host.StartRuntime(ctx, cfg)
}

func (l *Lifecycle) Attach(ctx context.Context, opts AttachOptions) (*runtimesdk.RuntimeHandle, error) {
	return l.host.AttachRuntime(ctx, runtimesdk.RuntimeHostAttachOptions{
		ControlEndpoint:    opts.ControlEndpoint,
		InvocationEndpoint: opts.InvocationEndpoint,
		ControlPath:        opts.ControlPath,
	})
}

func (l *Lifecycle) ConnectLocal(ctx context.Context, opts runtimesdk.ConnectOptions) (*runtimesdk.RuntimeClient, error) {
	return l.host.ConnectLocal(ctx, opts)
}
