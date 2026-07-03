package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
)

// DaemonMode is the local daemon deployment role.
type DaemonMode string

const (
	ModeDevice DaemonMode = "device"
	ModeHub    DaemonMode = "hub"
	ModeBoth   DaemonMode = "both"
)

// DaemonLifecycleState is the SDK daemon lifecycle state projection.
type DaemonLifecycleState string

const (
	DaemonUnknown          DaemonLifecycleState = "Unknown"
	DaemonDiscovered       DaemonLifecycleState = "Discovered"
	DaemonStarting         DaemonLifecycleState = "Starting"
	DaemonControlReady     DaemonLifecycleState = "ControlReady"
	DaemonInvocationReady  DaemonLifecycleState = "InvocationReady"
	DaemonRunning          DaemonLifecycleState = "Running"
	DaemonStopping         DaemonLifecycleState = "Stopping"
	DaemonStopped          DaemonLifecycleState = "Stopped"
	DaemonConfigInvalid    DaemonLifecycleState = "ConfigInvalid"
	DaemonPermissionDenied DaemonLifecycleState = "PermissionDenied"
	DaemonVersionMismatch  DaemonLifecycleState = "VersionMismatch"
	DaemonControlOnly      DaemonLifecycleState = "ControlOnly"
	DaemonInvocationDown   DaemonLifecycleState = "InvocationDown"
	DaemonStartFailed      DaemonLifecycleState = "StartFailed"
	DaemonCrashLoop        DaemonLifecycleState = "CrashLoop"
)

// StartConfig describes daemon lifecycle start policy.
type StartConfig struct {
	Mode        DaemonMode        `json:"mode"`
	Realm       string            `json:"realm,omitempty"`
	DeviceID    string            `json:"device_id,omitempty"`
	HomeDir     string            `json:"home_dir,omitempty"`
	DaemonBin   string            `json:"daemon_bin,omitempty"`
	LogPath     string            `json:"log_path,omitempty"`
	Detached    bool              `json:"detached,omitempty"`
	Env         map[string]string `json:"env,omitempty"`
	UDSPath     string            `json:"uds_path,omitempty"`
	ListenTCP   string            `json:"listen_tcp,omitempty"`
	TLSCertPath string            `json:"tls_cert_path,omitempty"`
	TLSKeyPath  string            `json:"tls_key_path,omitempty"`
	HubEndpoint string            `json:"hub_endpoint,omitempty"`
	TrustPath   string            `json:"trust_path,omitempty"`
}

// AttachOptions describes an existing daemon attachment request.
type AttachOptions struct {
	ControlEndpoint    string `json:"control_endpoint,omitempty"`
	InvocationEndpoint string `json:"invocation_endpoint,omitempty"`
	ControlPath        string `json:"control_path,omitempty"`
}

// DiscoverOptions describes daemon endpoint discovery knobs.
type DiscoverOptions struct {
	ControlEndpoint string `json:"control_endpoint,omitempty"`
	ControlPath     string `json:"control_path,omitempty"`
	HomeDir         string `json:"home_dir,omitempty"`
}

// StopOptions describes daemon stop policy.
type StopOptions struct {
	GracefulTimeoutMS int64 `json:"graceful_timeout_ms,omitempty"`
	Force             bool  `json:"force,omitempty"`
}

// Endpoints are daemon control and Invocation transport locators.
type Endpoints struct {
	ControlEndpoint    string `json:"control_endpoint,omitempty"`
	InvocationEndpoint string `json:"invocation_endpoint,omitempty"`
	PublicEndpoint     string `json:"public_endpoint,omitempty"`
}

// DaemonStatus is the typed daemon lifecycle status projection.
type DaemonStatus struct {
	HandleID    string               `json:"handle_id,omitempty"`
	State       DaemonLifecycleState `json:"state"`
	Mode        DaemonMode           `json:"mode,omitempty"`
	PID         int                  `json:"pid,omitempty"`
	Version     string               `json:"version,omitempty"`
	Message     string               `json:"message,omitempty"`
	Endpoints   Endpoints            `json:"endpoints,omitempty"`
	Diagnostics []string             `json:"diagnostics,omitempty"`
}

// DaemonTransport supplies daemon lifecycle operations behind the SDK facade.
type DaemonTransport interface {
	Discover(ctx context.Context, optionsJSON []byte) ([]byte, error)
	Start(ctx context.Context, configJSON []byte) ([]byte, error)
	Attach(ctx context.Context, optionsJSON []byte) ([]byte, error)
	Status(ctx context.Context, handleID string) ([]byte, error)
	OpenRuntime(ctx context.Context, handleID string, optionsJSON []byte) (RuntimeTransport, []byte, error)
	Stop(ctx context.Context, handleID string, optionsJSON []byte) ([]byte, error)
	Detach(ctx context.Context, handleID string) error
}

// DaemonTransportFunc adapts functions into a DaemonTransport.
type DaemonTransportFunc struct {
	DiscoverFunc    func(ctx context.Context, optionsJSON []byte) ([]byte, error)
	StartFunc       func(ctx context.Context, configJSON []byte) ([]byte, error)
	AttachFunc      func(ctx context.Context, optionsJSON []byte) ([]byte, error)
	StatusFunc      func(ctx context.Context, handleID string) ([]byte, error)
	OpenRuntimeFunc func(ctx context.Context, handleID string, optionsJSON []byte) (RuntimeTransport, []byte, error)
	StopFunc        func(ctx context.Context, handleID string, optionsJSON []byte) ([]byte, error)
	DetachFunc      func(ctx context.Context, handleID string) error
}

func (f DaemonTransportFunc) Discover(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if f.DiscoverFunc == nil {
		return nil, invalidRuntimeClient("daemon discover transport function is required")
	}
	return f.DiscoverFunc(ctx, optionsJSON)
}

func (f DaemonTransportFunc) Start(ctx context.Context, configJSON []byte) ([]byte, error) {
	if f.StartFunc == nil {
		return nil, invalidRuntimeClient("daemon start transport function is required")
	}
	return f.StartFunc(ctx, configJSON)
}

func (f DaemonTransportFunc) Attach(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if f.AttachFunc == nil {
		return nil, invalidRuntimeClient("daemon attach transport function is required")
	}
	return f.AttachFunc(ctx, optionsJSON)
}

func (f DaemonTransportFunc) Status(ctx context.Context, handleID string) ([]byte, error) {
	if f.StatusFunc == nil {
		return nil, invalidRuntimeClient("daemon status transport function is required")
	}
	return f.StatusFunc(ctx, handleID)
}

func (f DaemonTransportFunc) OpenRuntime(ctx context.Context, handleID string, optionsJSON []byte) (RuntimeTransport, []byte, error) {
	if f.OpenRuntimeFunc == nil {
		return nil, nil, invalidRuntimeClient("daemon open-runtime transport function is required")
	}
	return f.OpenRuntimeFunc(ctx, handleID, optionsJSON)
}

func (f DaemonTransportFunc) Stop(ctx context.Context, handleID string, optionsJSON []byte) ([]byte, error) {
	if f.StopFunc == nil {
		return nil, invalidRuntimeClient("daemon stop transport function is required")
	}
	return f.StopFunc(ctx, handleID, optionsJSON)
}

func (f DaemonTransportFunc) Detach(ctx context.Context, handleID string) error {
	if f.DetachFunc == nil {
		return nil
	}
	return f.DetachFunc(ctx, handleID)
}

// DaemonControl is the lifecycle facade root over an integration transport.
type DaemonControl struct {
	transport DaemonTransport
}

// NewDaemonControl creates a daemon lifecycle facade over a transport seam.
func NewDaemonControl(transport DaemonTransport) (*DaemonControl, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("daemon transport is required")
	}
	return &DaemonControl{transport: transport}, nil
}

// Discover returns daemon-advertised endpoints without hard-coded fallbacks.
func (c *DaemonControl) Discover(ctx context.Context, opts DiscoverOptions) (Endpoints, error) {
	if c == nil || c.transport == nil {
		return Endpoints{}, invalidRuntimeClient("daemon control is not initialized")
	}
	if ctx == nil {
		return Endpoints{}, invalidRuntimeClient("context is required")
	}
	optionsJSON, err := json.Marshal(opts)
	if err != nil {
		return Endpoints{}, invalidRuntimePayload(fmt.Sprintf("encode discover options: %v", err), err)
	}
	raw, err := c.transport.Discover(ctx, optionsJSON)
	if err != nil {
		return Endpoints{}, wrapDaemonTransportError("daemon discover failed", err)
	}
	return NewEndpointsFromJSON(raw, true)
}

// Start starts or adopts a daemon lifecycle handle once runtime traffic is ready.
func (c *DaemonControl) Start(ctx context.Context, cfg StartConfig) (*DaemonHandle, error) {
	if c == nil || c.transport == nil {
		return nil, invalidRuntimeClient("daemon control is not initialized")
	}
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	if err := validateStartConfig(cfg); err != nil {
		return nil, err
	}
	configJSON, err := json.Marshal(cfg)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode start config: %v", err), err)
	}
	raw, err := c.transport.Start(ctx, configJSON)
	if err != nil {
		return nil, wrapDaemonTransportError("daemon start failed", err)
	}
	status, err := NewDaemonStatusFromJSON(raw)
	if err != nil {
		return nil, err
	}
	if err := requireDaemonRuntimeReady(status); err != nil {
		return nil, err
	}
	return newDaemonHandle(c.transport, status)
}

// Attach attaches to an existing daemon only when Invocation traffic is ready.
func (c *DaemonControl) Attach(ctx context.Context, opts AttachOptions) (*DaemonHandle, error) {
	if c == nil || c.transport == nil {
		return nil, invalidRuntimeClient("daemon control is not initialized")
	}
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	optionsJSON, err := json.Marshal(opts)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode attach options: %v", err), err)
	}
	raw, err := c.transport.Attach(ctx, optionsJSON)
	if err != nil {
		return nil, wrapDaemonTransportError("daemon attach failed", err)
	}
	status, err := NewDaemonStatusFromJSON(raw)
	if err != nil {
		return nil, err
	}
	if err := requireDaemonRuntimeReady(status); err != nil {
		return nil, err
	}
	return newDaemonHandle(c.transport, status)
}

// ConnectLocal discovers a runtime-ready daemon, opens its Invocation runtime,
// and detaches the lifecycle handle without stopping the daemon.
func (c *DaemonControl) ConnectLocal(ctx context.Context, opts ConnectOptions) (*RuntimeClient, error) {
	if c == nil || c.transport == nil {
		return nil, invalidRuntimeClient("daemon control is not initialized")
	}
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	endpoints, err := c.Discover(ctx, DiscoverOptions{ControlPath: opts.ControlPath})
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
	handle, err := c.Attach(ctx, AttachOptions{
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

// DaemonHandle owns local daemon lifecycle handle state.
type DaemonHandle struct {
	transport DaemonTransport
	handleID  string
	status    DaemonStatus
	detached  bool
}

func newDaemonHandle(transport DaemonTransport, status DaemonStatus) (*DaemonHandle, error) {
	if status.HandleID == "" {
		return nil, invalidRuntimePayload("handle_id is required", nil)
	}
	return &DaemonHandle{
		transport: transport,
		handleID:  status.HandleID,
		status:    status,
	}, nil
}

func (h *DaemonHandle) Status(ctx context.Context) (DaemonStatus, error) {
	if err := h.requireAttached(); err != nil {
		return DaemonStatus{}, err
	}
	if ctx == nil {
		return DaemonStatus{}, invalidRuntimeClient("context is required")
	}
	raw, err := h.transport.Status(ctx, h.handleID)
	if err != nil {
		return DaemonStatus{}, wrapDaemonTransportError("daemon status failed", err)
	}
	status, err := NewDaemonStatusFromJSON(raw)
	if err != nil {
		return DaemonStatus{}, err
	}
	h.status = status
	return status, nil
}

func (h *DaemonHandle) Endpoints() Endpoints {
	if h == nil {
		return Endpoints{}
	}
	return h.status.Endpoints
}

func (h *DaemonHandle) OpenRuntime(ctx context.Context, opts ConnectOptions) (*RuntimeClient, error) {
	if err := h.requireAttached(); err != nil {
		return nil, err
	}
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	if !daemonRuntimeReady(h.status.State) {
		return nil, invalidRuntimePayload("daemon invocation endpoint is not ready", nil)
	}
	optionsJSON, err := json.Marshal(opts)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode runtime options: %v", err), err)
	}
	transport, _, err := h.transport.OpenRuntime(ctx, h.handleID, optionsJSON)
	if err != nil {
		return nil, wrapDaemonTransportError("daemon open runtime failed", err)
	}
	if transport == nil {
		return nil, invalidRuntimeClient("runtime transport is required")
	}
	return NewRuntimeClient(transport)
}

func (h *DaemonHandle) Stop(ctx context.Context, opts StopOptions) error {
	if err := h.requireAttached(); err != nil {
		return err
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	if h.status.State == DaemonStopped {
		return nil
	}
	optionsJSON, err := json.Marshal(opts)
	if err != nil {
		return invalidRuntimePayload(fmt.Sprintf("encode stop options: %v", err), err)
	}
	raw, err := h.transport.Stop(ctx, h.handleID, optionsJSON)
	if err != nil {
		return wrapDaemonTransportError("daemon stop failed", err)
	}
	status, err := NewDaemonStatusFromJSON(raw)
	if err != nil {
		return err
	}
	h.status = status
	return nil
}

func (h *DaemonHandle) Detach(ctx context.Context) error {
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
		return wrapDaemonTransportError("daemon detach failed", err)
	}
	h.detached = true
	return nil
}

func (h *DaemonHandle) HandleID() string {
	if h == nil {
		return ""
	}
	return h.handleID
}

func (h *DaemonHandle) State() DaemonLifecycleState {
	if h == nil {
		return DaemonUnknown
	}
	return h.status.State
}

func (h *DaemonHandle) requireAttached() error {
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

// Start starts a daemon through an explicit lifecycle transport seam.
func Start(ctx context.Context, transport DaemonTransport, cfg StartConfig) (*DaemonHandle, error) {
	control, err := NewDaemonControl(transport)
	if err != nil {
		return nil, err
	}
	return control.Start(ctx, cfg)
}

// Attach attaches to an existing daemon through an explicit lifecycle transport seam.
func Attach(ctx context.Context, transport DaemonTransport, opts AttachOptions) (*DaemonHandle, error) {
	control, err := NewDaemonControl(transport)
	if err != nil {
		return nil, err
	}
	return control.Attach(ctx, opts)
}

// Discover returns daemon-advertised endpoints through an explicit transport seam.
func Discover(ctx context.Context, transport DaemonTransport, opts DiscoverOptions) (Endpoints, error) {
	control, err := NewDaemonControl(transport)
	if err != nil {
		return Endpoints{}, err
	}
	return control.Discover(ctx, opts)
}

// ConnectLocal discovers, attaches, opens, and detaches a local daemon runtime.
func ConnectLocal(ctx context.Context, transport DaemonTransport, opts ConnectOptions) (*RuntimeClient, error) {
	control, err := NewDaemonControl(transport)
	if err != nil {
		return nil, err
	}
	return control.ConnectLocal(ctx, opts)
}

func validateStartConfig(cfg StartConfig) error {
	switch cfg.Mode {
	case ModeDevice:
		if cfg.ListenTCP != "" {
			return invalidRuntimePayload("device mode must not accept a public TCP listener", nil)
		}
	case ModeHub, ModeBoth:
		if cfg.ListenTCP != "" && (cfg.TLSCertPath == "" || cfg.TLSKeyPath == "") {
			return invalidRuntimePayload("public TCP listener requires TLS material", nil)
		}
	default:
		return invalidRuntimePayload("mode must be device, hub, or both", nil)
	}
	return nil
}

func requireDaemonRuntimeReady(status DaemonStatus) error {
	if daemonRuntimeReady(status.State) {
		return nil
	}
	if status.State == DaemonControlOnly || status.State == DaemonControlReady {
		return &SDKError{
			Code:      ErrControlOnly,
			Stage:     "daemon_lifecycle",
			Retry:     RetrySafe,
			Retryable: true,
			Message:   "daemon control endpoint is ready but invocation endpoint is not ready",
		}
	}
	return invalidRuntimePayload("daemon invocation endpoint is not ready", nil)
}

func daemonRuntimeReady(state DaemonLifecycleState) bool {
	return state == DaemonInvocationReady || state == DaemonRunning
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

func NewDaemonStatusFromJSON(raw []byte) (DaemonStatus, error) {
	var status DaemonStatus
	if err := json.Unmarshal(raw, &status); err != nil {
		return DaemonStatus{}, invalidRuntimePayload(fmt.Sprintf("decode daemon status JSON: %v", err), err)
	}
	if status.State == "" {
		return DaemonStatus{}, invalidRuntimePayload("state is required", nil)
	}
	if !validDaemonState(status.State) {
		return DaemonStatus{}, invalidRuntimePayload("invalid daemon lifecycle state", nil)
	}
	return status, nil
}

func validDaemonState(state DaemonLifecycleState) bool {
	switch state {
	case DaemonUnknown, DaemonDiscovered, DaemonStarting, DaemonControlReady,
		DaemonInvocationReady, DaemonRunning, DaemonStopping, DaemonStopped,
		DaemonConfigInvalid, DaemonPermissionDenied, DaemonVersionMismatch,
		DaemonControlOnly, DaemonInvocationDown, DaemonStartFailed, DaemonCrashLoop:
		return true
	default:
		return false
	}
}

func wrapDaemonTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
