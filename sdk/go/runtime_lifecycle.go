package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
)

// RuntimeMode is an opaque provider-reported runtime host role.
type RuntimeMode string

// RuntimeHostMode is the canonical runtime-host role projection.
type RuntimeHostMode = RuntimeMode

// RuntimeLifecycleState is the SDK runtime lifecycle state projection.
type RuntimeLifecycleState string

const (
	RuntimeUnknown          RuntimeLifecycleState = "Unknown"
	RuntimeDiscovered       RuntimeLifecycleState = "Discovered"
	RuntimeStarting         RuntimeLifecycleState = "Starting"
	RuntimeControlReady     RuntimeLifecycleState = "ControlReady"
	RuntimeInvocationReady  RuntimeLifecycleState = "InvocationReady"
	RuntimeRunning          RuntimeLifecycleState = "Running"
	RuntimeStopping         RuntimeLifecycleState = "Stopping"
	RuntimeStopped          RuntimeLifecycleState = "Stopped"
	RuntimeConfigInvalid    RuntimeLifecycleState = "ConfigInvalid"
	RuntimePermissionDenied RuntimeLifecycleState = "PermissionDenied"
	RuntimeVersionMismatch  RuntimeLifecycleState = "VersionMismatch"
	RuntimeControlOnly      RuntimeLifecycleState = "ControlOnly"
	RuntimeInvocationDown   RuntimeLifecycleState = "InvocationDown"
	RuntimeStartFailed      RuntimeLifecycleState = "StartFailed"
	RuntimeCrashLoop        RuntimeLifecycleState = "CrashLoop"
)

// RuntimeHostStartRequest is a provider-owned, validated runtime-host start
// request. The canonical lifecycle owns state transitions, not process policy.
type RuntimeHostStartRequest interface {
	Validate() error
	RuntimeHostStartPayload() ([]byte, error)
}

// RuntimeHostDiscoverRequest is a provider-owned runtime-host discovery
// request. Product directory roots remain outside the canonical lifecycle.
type RuntimeHostDiscoverRequest interface {
	RuntimeHostDiscoverPayload() ([]byte, error)
}

type runtimeEndpointDiscoveryRequest struct {
	ControlEndpoint string `json:"control_endpoint,omitempty"`
	ControlPath     string `json:"control_path,omitempty"`
}

func (o runtimeEndpointDiscoveryRequest) RuntimeHostDiscoverPayload() ([]byte, error) {
	return json.Marshal(o)
}

// RuntimeHostDiscoverOptions identifies a runtime host by canonical endpoint
// locators. Product directory roots remain provider-owned request fields.
type RuntimeHostDiscoverOptions struct {
	ControlEndpoint string `json:"control_endpoint,omitempty"`
	ControlPath     string `json:"control_path,omitempty"`
}

func (o RuntimeHostDiscoverOptions) RuntimeHostDiscoverPayload() ([]byte, error) {
	return json.Marshal(o)
}

// RuntimeHostAttachOptions identifies an existing runtime host.
type RuntimeHostAttachOptions struct {
	ControlEndpoint    string `json:"control_endpoint,omitempty"`
	InvocationEndpoint string `json:"invocation_endpoint,omitempty"`
	ControlPath        string `json:"control_path,omitempty"`
}

// RuntimeHostStopOptions controls runtime-host shutdown.
type RuntimeHostStopOptions struct {
	GracefulTimeoutMS int64 `json:"graceful_timeout_ms,omitempty"`
	Force             bool  `json:"force,omitempty"`
}

// Endpoints are daemon control and Invocation transport locators.
type Endpoints struct {
	ControlEndpoint    string `json:"control_endpoint,omitempty"`
	InvocationEndpoint string `json:"invocation_endpoint,omitempty"`
	PublicEndpoint     string `json:"public_endpoint,omitempty"`
}

// RuntimeHostEndpoints are the canonical runtime-host endpoint locators.
type RuntimeHostEndpoints = Endpoints

// RuntimeLifecycleStatus is the typed runtime lifecycle status projection.
type RuntimeLifecycleStatus struct {
	HandleID    string                `json:"handle_id,omitempty"`
	State       RuntimeLifecycleState `json:"state"`
	Mode        RuntimeMode           `json:"mode,omitempty"`
	PID         int                   `json:"pid,omitempty"`
	Version     string                `json:"version,omitempty"`
	Message     string                `json:"message,omitempty"`
	Endpoints   Endpoints             `json:"endpoints,omitempty"`
	Diagnostics []string              `json:"diagnostics,omitempty"`
}

// RuntimeHostStatus is the canonical runtime-host status projection.
type RuntimeHostStatus = RuntimeLifecycleStatus

// RuntimeLifecycleTransport supplies runtime lifecycle operations behind the SDK facade.
type RuntimeLifecycleTransport interface {
	Discover(ctx context.Context, optionsJSON []byte) ([]byte, error)
	Start(ctx context.Context, configJSON []byte) ([]byte, error)
	Attach(ctx context.Context, optionsJSON []byte) ([]byte, error)
	Status(ctx context.Context, handleID string) ([]byte, error)
	OpenRuntime(ctx context.Context, handleID string, optionsJSON []byte) (RuntimeTransport, []byte, error)
	Stop(ctx context.Context, handleID string, optionsJSON []byte) ([]byte, error)
	Detach(ctx context.Context, handleID string) error
}

// RuntimeLifecycleTransportFunc adapts functions into a RuntimeLifecycleTransport.
type RuntimeLifecycleTransportFunc struct {
	DiscoverFunc    func(ctx context.Context, optionsJSON []byte) ([]byte, error)
	StartFunc       func(ctx context.Context, configJSON []byte) ([]byte, error)
	AttachFunc      func(ctx context.Context, optionsJSON []byte) ([]byte, error)
	StatusFunc      func(ctx context.Context, handleID string) ([]byte, error)
	OpenRuntimeFunc func(ctx context.Context, handleID string, optionsJSON []byte) (RuntimeTransport, []byte, error)
	StopFunc        func(ctx context.Context, handleID string, optionsJSON []byte) ([]byte, error)
	DetachFunc      func(ctx context.Context, handleID string) error
}

func (f RuntimeLifecycleTransportFunc) Discover(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if f.DiscoverFunc == nil {
		return nil, invalidRuntimeClient("daemon discover transport function is required")
	}
	return f.DiscoverFunc(ctx, optionsJSON)
}

func (f RuntimeLifecycleTransportFunc) Start(ctx context.Context, configJSON []byte) ([]byte, error) {
	if f.StartFunc == nil {
		return nil, invalidRuntimeClient("daemon start transport function is required")
	}
	return f.StartFunc(ctx, configJSON)
}

func (f RuntimeLifecycleTransportFunc) Attach(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if f.AttachFunc == nil {
		return nil, invalidRuntimeClient("daemon attach transport function is required")
	}
	return f.AttachFunc(ctx, optionsJSON)
}

func (f RuntimeLifecycleTransportFunc) Status(ctx context.Context, handleID string) ([]byte, error) {
	if f.StatusFunc == nil {
		return nil, invalidRuntimeClient("daemon status transport function is required")
	}
	return f.StatusFunc(ctx, handleID)
}

func (f RuntimeLifecycleTransportFunc) OpenRuntime(ctx context.Context, handleID string, optionsJSON []byte) (RuntimeTransport, []byte, error) {
	if f.OpenRuntimeFunc == nil {
		return nil, nil, invalidRuntimeClient("daemon open-runtime transport function is required")
	}
	return f.OpenRuntimeFunc(ctx, handleID, optionsJSON)
}

func (f RuntimeLifecycleTransportFunc) Stop(ctx context.Context, handleID string, optionsJSON []byte) ([]byte, error) {
	if f.StopFunc == nil {
		return nil, invalidRuntimeClient("daemon stop transport function is required")
	}
	return f.StopFunc(ctx, handleID, optionsJSON)
}

func (f RuntimeLifecycleTransportFunc) Detach(ctx context.Context, handleID string) error {
	if f.DetachFunc == nil {
		return nil
	}
	return f.DetachFunc(ctx, handleID)
}

// RuntimeHostLifecycle is the provider-neutral lifecycle facade.
type RuntimeHostLifecycle interface {
	DiscoverRuntime(ctx context.Context, request RuntimeHostDiscoverRequest) (Endpoints, error)
	StartRuntime(ctx context.Context, request RuntimeHostStartRequest) (*RuntimeHandle, error)
	AttachRuntime(ctx context.Context, opts RuntimeHostAttachOptions) (*RuntimeHandle, error)
	ConnectLocal(ctx context.Context, opts ConnectOptions) (*RuntimeClient, error)
}

// RuntimeHost is the lifecycle facade root over an integration transport.
type RuntimeHost struct {
	transport RuntimeLifecycleTransport
}

// NewRuntimeHost creates a runtime lifecycle facade over a transport seam.
func NewRuntimeHost(transport RuntimeLifecycleTransport) (*RuntimeHost, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("daemon transport is required")
	}
	return &RuntimeHost{transport: transport}, nil
}

// DiscoverRuntime returns provider-advertised endpoints without hard-coded fallbacks.
func (h *RuntimeHost) DiscoverRuntime(ctx context.Context, request RuntimeHostDiscoverRequest) (Endpoints, error) {
	if h == nil || h.transport == nil {
		return Endpoints{}, invalidRuntimeClient("daemon control is not initialized")
	}
	if ctx == nil {
		return Endpoints{}, invalidRuntimeClient("context is required")
	}
	if request == nil {
		return Endpoints{}, invalidRuntimeClient("runtime host discover request is required")
	}
	optionsJSON, err := request.RuntimeHostDiscoverPayload()
	if err != nil {
		return Endpoints{}, invalidRuntimePayload(fmt.Sprintf("encode discover options: %v", err), err)
	}
	raw, err := h.transport.Discover(ctx, optionsJSON)
	if err != nil {
		return Endpoints{}, wrapRuntimeLifecycleTransportError("daemon discover failed", err)
	}
	return NewEndpointsFromJSON(raw, true)
}

// StartRuntime starts or adopts a runtime lifecycle handle once runtime traffic is ready.
func (h *RuntimeHost) StartRuntime(ctx context.Context, request RuntimeHostStartRequest) (*RuntimeHandle, error) {
	if h == nil || h.transport == nil {
		return nil, invalidRuntimeClient("daemon control is not initialized")
	}
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	if request == nil {
		return nil, invalidRuntimeClient("runtime host start request is required")
	}
	if err := request.Validate(); err != nil {
		return nil, err
	}
	configJSON, err := request.RuntimeHostStartPayload()
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode start config: %v", err), err)
	}
	raw, err := h.transport.Start(ctx, configJSON)
	if err != nil {
		return nil, wrapRuntimeLifecycleTransportError("daemon start failed", err)
	}
	status, err := NewRuntimeLifecycleStatusFromJSON(raw)
	if err != nil {
		return nil, err
	}
	if err := requireRuntimeLifecycleReady(status); err != nil {
		return nil, err
	}
	return newRuntimeHandle(h.transport, status)
}

// AttachRuntime attaches to an existing runtime host only when Invocation traffic is ready.
func (h *RuntimeHost) AttachRuntime(ctx context.Context, opts RuntimeHostAttachOptions) (*RuntimeHandle, error) {
	if h == nil || h.transport == nil {
		return nil, invalidRuntimeClient("daemon control is not initialized")
	}
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	optionsJSON, err := json.Marshal(opts)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode attach options: %v", err), err)
	}
	raw, err := h.transport.Attach(ctx, optionsJSON)
	if err != nil {
		return nil, wrapRuntimeLifecycleTransportError("daemon attach failed", err)
	}
	status, err := NewRuntimeLifecycleStatusFromJSON(raw)
	if err != nil {
		return nil, err
	}
	if err := requireRuntimeLifecycleReady(status); err != nil {
		return nil, err
	}
	return newRuntimeHandle(h.transport, status)
}

// ConnectLocal discovers a runtime-ready daemon, opens its Invocation runtime,
// and detaches the lifecycle handle without stopping the daemon.
func (h *RuntimeHost) ConnectLocal(ctx context.Context, opts ConnectOptions) (*RuntimeClient, error) {
	if h == nil || h.transport == nil {
		return nil, invalidRuntimeClient("daemon control is not initialized")
	}
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	endpoints, err := h.DiscoverRuntime(ctx, runtimeEndpointDiscoveryRequest{ControlPath: opts.ControlPath})
	if err != nil {
		return nil, err
	}
	runtimeEndpoint := endpoints.InvocationEndpoint
	if opts.Endpoint != "" {
		runtimeEndpoint = opts.Endpoint
	}
	if runtimeEndpoint == "" {
		return nil, invalidRuntimePayload("invocation_endpoint is required", nil)
	}
	handle, err := h.AttachRuntime(ctx, RuntimeHostAttachOptions{
		ControlEndpoint:    endpoints.ControlEndpoint,
		InvocationEndpoint: runtimeEndpoint,
		ControlPath:        opts.ControlPath,
	})
	if err != nil {
		return nil, err
	}
	openOptions := opts
	openOptions.Endpoint = runtimeEndpoint
	client, err := handle.OpenRuntime(ctx, openOptions)
	detachErr := handle.Detach(ctx)
	if err != nil {
		return nil, errors.Join(err, detachErr)
	}
	if detachErr != nil {
		return nil, detachErr
	}
	return client, nil
}

// RuntimeHandle owns local runtime lifecycle handle state.
type RuntimeHandle struct {
	transport RuntimeLifecycleTransport
	handleID  string
	status    RuntimeLifecycleStatus
	detached  bool
}

func newRuntimeHandle(transport RuntimeLifecycleTransport, status RuntimeLifecycleStatus) (*RuntimeHandle, error) {
	if status.HandleID == "" {
		return nil, invalidRuntimePayload("handle_id is required", nil)
	}
	return &RuntimeHandle{
		transport: transport,
		handleID:  status.HandleID,
		status:    status,
	}, nil
}

func (h *RuntimeHandle) Status(ctx context.Context) (RuntimeLifecycleStatus, error) {
	if err := h.requireAttached(); err != nil {
		return RuntimeLifecycleStatus{}, err
	}
	if ctx == nil {
		return RuntimeLifecycleStatus{}, invalidRuntimeClient("context is required")
	}
	raw, err := h.transport.Status(ctx, h.handleID)
	if err != nil {
		return RuntimeLifecycleStatus{}, wrapRuntimeLifecycleTransportError("daemon status failed", err)
	}
	status, err := NewRuntimeLifecycleStatusFromJSON(raw)
	if err != nil {
		return RuntimeLifecycleStatus{}, err
	}
	if err := validateRuntimeLifecycleTransition(h.status.State, status.State); err != nil {
		return RuntimeLifecycleStatus{}, err
	}
	h.status = status
	return status, nil
}

func (h *RuntimeHandle) Endpoints() Endpoints {
	if h == nil {
		return Endpoints{}
	}
	return h.status.Endpoints
}

func (h *RuntimeHandle) OpenRuntime(ctx context.Context, opts ConnectOptions) (*RuntimeClient, error) {
	if err := h.requireAttached(); err != nil {
		return nil, err
	}
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	if !runtimeLifecycleReady(h.status.State) {
		return nil, invalidRuntimePayload("daemon invocation endpoint is not ready", nil)
	}
	optionsJSON, err := json.Marshal(opts)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode runtime options: %v", err), err)
	}
	transport, _, err := h.transport.OpenRuntime(ctx, h.handleID, optionsJSON)
	if err != nil {
		return nil, wrapRuntimeLifecycleTransportError("daemon open runtime failed", err)
	}
	if transport == nil {
		return nil, invalidRuntimeClient("runtime transport is required")
	}
	return NewRuntimeClient(transport)
}

func (h *RuntimeHandle) StopRuntime(ctx context.Context, opts RuntimeHostStopOptions) error {
	if err := h.requireAttached(); err != nil {
		return err
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	if h.status.State == RuntimeStopped {
		return nil
	}
	optionsJSON, err := json.Marshal(opts)
	if err != nil {
		return invalidRuntimePayload(fmt.Sprintf("encode stop options: %v", err), err)
	}
	raw, err := h.transport.Stop(ctx, h.handleID, optionsJSON)
	if err != nil {
		return wrapRuntimeLifecycleTransportError("daemon stop failed", err)
	}
	status, err := NewRuntimeLifecycleStatusFromJSON(raw)
	if err != nil {
		return err
	}
	if err := validateRuntimeLifecycleTransition(h.status.State, status.State); err != nil {
		return err
	}
	h.status = status
	return nil
}

func (h *RuntimeHandle) Detach(ctx context.Context) error {
	if h == nil || h.transport == nil {
		return invalidRuntimeClient("daemon handle is not initialized")
	}
	if h.detached {
		return nil
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	if err := h.transport.Detach(ctx, h.handleID); err != nil {
		return wrapRuntimeLifecycleTransportError("daemon detach failed", err)
	}
	h.detached = true
	return nil
}

func (h *RuntimeHandle) HandleID() string {
	if h == nil {
		return ""
	}
	return h.handleID
}

func (h *RuntimeHandle) State() RuntimeLifecycleState {
	if h == nil {
		return RuntimeUnknown
	}
	return h.status.State
}

func (h *RuntimeHandle) requireAttached() error {
	if h == nil || h.transport == nil {
		return invalidRuntimeClient("daemon handle is not initialized")
	}
	if h.detached {
		return &SDKError{
			Code:      ErrInvalidHandle,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "daemon handle is detached",
		}
	}
	return nil
}

// StartRuntimeHost starts a runtime host through an explicit lifecycle transport seam.
func StartRuntimeHost(ctx context.Context, transport RuntimeLifecycleTransport, request RuntimeHostStartRequest) (*RuntimeHandle, error) {
	control, err := NewRuntimeHost(transport)
	if err != nil {
		return nil, err
	}
	return control.StartRuntime(ctx, request)
}

// AttachRuntimeHost attaches to an existing runtime host through an explicit lifecycle transport seam.
func AttachRuntimeHost(ctx context.Context, transport RuntimeLifecycleTransport, opts RuntimeHostAttachOptions) (*RuntimeHandle, error) {
	control, err := NewRuntimeHost(transport)
	if err != nil {
		return nil, err
	}
	return control.AttachRuntime(ctx, opts)
}

// DiscoverRuntimeHost returns runtime-host-advertised endpoints through an explicit transport seam.
func DiscoverRuntimeHost(ctx context.Context, transport RuntimeLifecycleTransport, request RuntimeHostDiscoverRequest) (RuntimeHostEndpoints, error) {
	control, err := NewRuntimeHost(transport)
	if err != nil {
		return Endpoints{}, err
	}
	return control.DiscoverRuntime(ctx, request)
}

// ConnectLocalRuntimeHost discovers, attaches, opens, and detaches a local runtime host.
func ConnectLocalRuntimeHost(ctx context.Context, transport RuntimeLifecycleTransport, opts ConnectOptions) (*RuntimeClient, error) {
	control, err := NewRuntimeHost(transport)
	if err != nil {
		return nil, err
	}
	return control.ConnectLocal(ctx, opts)
}

func requireRuntimeLifecycleReady(status RuntimeLifecycleStatus) error {
	if runtimeLifecycleReady(status.State) {
		return nil
	}
	if status.State == RuntimeControlOnly || status.State == RuntimeControlReady {
		return &SDKError{
			Code:      ErrControlOnly,
			Stage:     "runtime_lifecycle",
			Retry:     RetrySafe,
			Retryable: true,
			Message:   "daemon control endpoint is ready but invocation endpoint is not ready",
		}
	}
	return invalidRuntimePayload("daemon invocation endpoint is not ready", nil)
}

func runtimeLifecycleReady(state RuntimeLifecycleState) bool {
	return state == RuntimeInvocationReady || state == RuntimeRunning
}

func NewEndpointsFromJSON(raw []byte, requireInvocation bool) (Endpoints, error) {
	var endpoints Endpoints
	if err := json.Unmarshal(raw, &endpoints); err != nil {
		return Endpoints{}, invalidRuntimePayload(fmt.Sprintf("decode daemon endpoints JSON: %v", err), err)
	}
	if requireInvocation && endpoints.InvocationEndpoint == "" {
		return Endpoints{}, invalidRuntimePayload("invocation_endpoint is required", nil)
	}
	return endpoints, nil
}

func NewRuntimeLifecycleStatusFromJSON(raw []byte) (RuntimeLifecycleStatus, error) {
	var status RuntimeLifecycleStatus
	if err := json.Unmarshal(raw, &status); err != nil {
		return RuntimeLifecycleStatus{}, invalidRuntimePayload(fmt.Sprintf("decode daemon status JSON: %v", err), err)
	}
	if status.State == "" {
		return RuntimeLifecycleStatus{}, invalidRuntimePayload("state is required", nil)
	}
	if !validRuntimeLifecycleState(status.State) {
		return RuntimeLifecycleStatus{}, invalidRuntimePayload("invalid daemon lifecycle state", nil)
	}
	return status, nil
}

func validRuntimeLifecycleState(state RuntimeLifecycleState) bool {
	switch state {
	case RuntimeUnknown, RuntimeDiscovered, RuntimeStarting, RuntimeControlReady,
		RuntimeInvocationReady, RuntimeRunning, RuntimeStopping, RuntimeStopped,
		RuntimeConfigInvalid, RuntimePermissionDenied, RuntimeVersionMismatch,
		RuntimeControlOnly, RuntimeInvocationDown, RuntimeStartFailed, RuntimeCrashLoop:
		return true
	default:
		return false
	}
}

func validateRuntimeLifecycleTransition(current, next RuntimeLifecycleState) error {
	if current == next || current == RuntimeUnknown {
		return nil
	}
	allowed := map[RuntimeLifecycleState]map[RuntimeLifecycleState]bool{
		RuntimeDiscovered:      {RuntimeStarting: true, RuntimeControlReady: true, RuntimeInvocationReady: true, RuntimeRunning: true, RuntimeControlOnly: true, RuntimeStartFailed: true},
		RuntimeStarting:        {RuntimeControlReady: true, RuntimeInvocationReady: true, RuntimeRunning: true, RuntimeControlOnly: true, RuntimeStartFailed: true, RuntimeConfigInvalid: true, RuntimePermissionDenied: true, RuntimeVersionMismatch: true},
		RuntimeControlReady:    {RuntimeInvocationReady: true, RuntimeRunning: true, RuntimeControlOnly: true, RuntimeStopping: true, RuntimeStartFailed: true},
		RuntimeInvocationReady: {RuntimeRunning: true, RuntimeControlOnly: true, RuntimeInvocationDown: true, RuntimeStopping: true, RuntimeCrashLoop: true},
		RuntimeRunning:         {RuntimeControlOnly: true, RuntimeInvocationDown: true, RuntimeStopping: true, RuntimeStopped: true, RuntimeCrashLoop: true},
		RuntimeControlOnly:     {RuntimeInvocationReady: true, RuntimeRunning: true, RuntimeStopping: true, RuntimeCrashLoop: true},
		RuntimeInvocationDown:  {RuntimeInvocationReady: true, RuntimeRunning: true, RuntimeControlOnly: true, RuntimeStopping: true, RuntimeCrashLoop: true},
		RuntimeStopping:        {RuntimeStopped: true, RuntimeCrashLoop: true},
	}
	if allowed[current][next] {
		return nil
	}
	return invalidRuntimePayload(fmt.Sprintf("runtime lifecycle cannot transition from %s to %s", current, next), nil)
}

func wrapRuntimeLifecycleTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
