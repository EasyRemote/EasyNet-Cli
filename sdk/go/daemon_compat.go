package easynet

import (
	"context"

	easynetprovider "easynet.run/cli/sdk/go/provider/easynet/contract"
)

// REQ-LANG-5 compatibility types are exact EasyNet provider aliases. This
// module owns no lifecycle state machine, transport, or fallback behavior.
type DaemonMode = easynetprovider.Mode
type StartConfig = easynetprovider.StartConfig
type RuntimeHostStartConfig = StartConfig
type AttachOptions = easynetprovider.AttachOptions
type DiscoverOptions = easynetprovider.DiscoverOptions
type RuntimeHostDiscoverOptions = DiscoverOptions
type StopOptions = easynetprovider.StopOptions

const (
	RuntimeModeDevice = "device"
	RuntimeModeHub    = "hub"
	RuntimeModeBoth   = "both"

	ModeDevice = "device"
	ModeHub    = "hub"
	ModeBoth   = "both"

	DaemonUnknown          = RuntimeUnknown
	DaemonDiscovered       = RuntimeDiscovered
	DaemonStarting         = RuntimeStarting
	DaemonControlReady     = RuntimeControlReady
	DaemonInvocationReady  = RuntimeInvocationReady
	DaemonRunning          = RuntimeRunning
	DaemonStopping         = RuntimeStopping
	DaemonStopped          = RuntimeStopped
	DaemonConfigInvalid    = RuntimeConfigInvalid
	DaemonPermissionDenied = RuntimePermissionDenied
	DaemonVersionMismatch  = RuntimeVersionMismatch
	DaemonControlOnly      = RuntimeControlOnly
	DaemonInvocationDown   = RuntimeInvocationDown
	DaemonStartFailed      = RuntimeStartFailed
	DaemonCrashLoop        = RuntimeCrashLoop
)

type DaemonLifecycleState = RuntimeLifecycleState
type DaemonStatus = RuntimeLifecycleStatus
type DaemonTransport = RuntimeLifecycleTransport
type DaemonTransportFunc = RuntimeLifecycleTransportFunc
type DaemonControl = RuntimeHost
type DaemonHandle = RuntimeHandle

// RuntimeLifecycle is the source-compatible EasyNet provider facade.
type RuntimeLifecycle interface {
	Discover(ctx context.Context, opts DiscoverOptions) (Endpoints, error)
	Start(ctx context.Context, cfg StartConfig) (*RuntimeHandle, error)
	Attach(ctx context.Context, opts AttachOptions) (*RuntimeHandle, error)
	ConnectLocal(ctx context.Context, opts ConnectOptions) (*RuntimeClient, error)
}

type runtimeLifecycleCompatibilityAdapter struct {
	legacy RuntimeLifecycle
}

func (a runtimeLifecycleCompatibilityAdapter) DiscoverRuntime(ctx context.Context, request RuntimeHostDiscoverRequest) (Endpoints, error) {
	switch value := request.(type) {
	case DiscoverOptions:
		return a.legacy.Discover(ctx, value)
	default:
		return Endpoints{}, invalidRuntimeClient("unsupported runtime host discover request")
	}
}

func (a runtimeLifecycleCompatibilityAdapter) StartRuntime(ctx context.Context, request RuntimeHostStartRequest) (*RuntimeHandle, error) {
	config, ok := request.(StartConfig)
	if !ok {
		return nil, invalidRuntimeClient("unsupported runtime host start request")
	}
	return a.legacy.Start(ctx, config)
}

func (a runtimeLifecycleCompatibilityAdapter) AttachRuntime(ctx context.Context, opts RuntimeHostAttachOptions) (*RuntimeHandle, error) {
	return a.legacy.Attach(ctx, AttachOptions{
		ControlEndpoint:    opts.ControlEndpoint,
		InvocationEndpoint: opts.InvocationEndpoint,
		ControlPath:        opts.ControlPath,
	})
}

func (a runtimeLifecycleCompatibilityAdapter) ConnectLocal(ctx context.Context, opts ConnectOptions) (*RuntimeClient, error) {
	return a.legacy.ConnectLocal(ctx, opts)
}

func NewDaemonControl(transport DaemonTransport) (*DaemonControl, error) {
	return NewRuntimeHost(transport)
}

func (h *RuntimeHost) Discover(ctx context.Context, opts DiscoverOptions) (Endpoints, error) {
	return h.DiscoverRuntime(ctx, opts)
}

func (h *RuntimeHost) Start(ctx context.Context, cfg StartConfig) (*RuntimeHandle, error) {
	return h.StartRuntime(ctx, cfg)
}

func (h *RuntimeHost) Attach(ctx context.Context, opts AttachOptions) (*RuntimeHandle, error) {
	return h.AttachRuntime(ctx, RuntimeHostAttachOptions{
		ControlEndpoint:    opts.ControlEndpoint,
		InvocationEndpoint: opts.InvocationEndpoint,
		ControlPath:        opts.ControlPath,
	})
}

func (h *RuntimeHandle) Stop(ctx context.Context, opts StopOptions) error {
	return h.StopRuntime(ctx, RuntimeHostStopOptions{
		GracefulTimeoutMS: opts.GracefulTimeoutMS,
		Force:             opts.Force,
	})
}

func Start(ctx context.Context, transport RuntimeLifecycleTransport, cfg StartConfig) (*RuntimeHandle, error) {
	return StartRuntimeHost(ctx, transport, cfg)
}

func Attach(ctx context.Context, transport RuntimeLifecycleTransport, opts AttachOptions) (*RuntimeHandle, error) {
	control, err := NewRuntimeHost(transport)
	if err != nil {
		return nil, err
	}
	return control.Attach(ctx, opts)
}

func Discover(ctx context.Context, transport RuntimeLifecycleTransport, opts DiscoverOptions) (Endpoints, error) {
	return DiscoverRuntimeHost(ctx, transport, opts)
}

func ConnectLocal(ctx context.Context, transport RuntimeLifecycleTransport, opts ConnectOptions) (*RuntimeClient, error) {
	return ConnectLocalRuntimeHost(ctx, transport, opts)
}
