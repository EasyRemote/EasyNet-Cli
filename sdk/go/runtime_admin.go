package easynet

import "context"

type RuntimeAdminCommand string

const (
	RuntimeAdminDiscover    RuntimeAdminCommand = "Discover"
	RuntimeAdminStart       RuntimeAdminCommand = "Start"
	RuntimeAdminAttach      RuntimeAdminCommand = "Attach"
	RuntimeAdminStatus      RuntimeAdminCommand = "Status"
	RuntimeAdminOpenRuntime RuntimeAdminCommand = "OpenRuntime"
	RuntimeAdminStop        RuntimeAdminCommand = "Stop"
	RuntimeAdminDetach      RuntimeAdminCommand = "Detach"
	RuntimeAdminHealth      RuntimeAdminCommand = "Health"
	RuntimeAdminDiagnostics RuntimeAdminCommand = "Diagnostics"
)

type RuntimeReadiness struct {
	LifecycleState DaemonLifecycleState `json:"lifecycle_state"`
	Endpoints      Endpoints            `json:"endpoints"`
	Health         RuntimeHealth        `json:"health"`
	Diagnostics    *DiagnosticsReport   `json:"diagnostics,omitempty"`
	Ready          bool                 `json:"ready"`
	Messages       []string             `json:"messages,omitempty"`
}

type RuntimeAdminClient struct {
	control *DaemonControl
	health  *HealthClient
}

func NewRuntimeAdminClient(control *DaemonControl, health *HealthClient) (*RuntimeAdminClient, error) {
	if control == nil {
		return nil, invalidRuntimeClient("daemon control is required")
	}
	return &RuntimeAdminClient{control: control, health: health}, nil
}

func (c *RuntimeAdminClient) Discover(ctx context.Context, opts DiscoverOptions) (Endpoints, error) {
	control, err := c.requireControl(ctx)
	if err != nil {
		return Endpoints{}, err
	}
	return control.Discover(ctx, opts)
}

func (c *RuntimeAdminClient) Start(ctx context.Context, cfg StartConfig) (*DaemonHandle, error) {
	control, err := c.requireControl(ctx)
	if err != nil {
		return nil, err
	}
	return control.Start(ctx, cfg)
}

func (c *RuntimeAdminClient) Attach(ctx context.Context, opts AttachOptions) (*DaemonHandle, error) {
	control, err := c.requireControl(ctx)
	if err != nil {
		return nil, err
	}
	return control.Attach(ctx, opts)
}

func (c *RuntimeAdminClient) Status(ctx context.Context, handle *DaemonHandle) (DaemonStatus, error) {
	if _, err := c.requireControl(ctx); err != nil {
		return DaemonStatus{}, err
	}
	if handle == nil {
		return DaemonStatus{}, invalidRuntimeClient("daemon handle is required")
	}
	return handle.Status(ctx)
}

func (c *RuntimeAdminClient) OpenRuntime(ctx context.Context, handle *DaemonHandle, opts ConnectOptions) (*RuntimeClient, error) {
	if _, err := c.requireControl(ctx); err != nil {
		return nil, err
	}
	if handle == nil {
		return nil, invalidRuntimeClient("daemon handle is required")
	}
	return handle.OpenRuntime(ctx, opts)
}

func (c *RuntimeAdminClient) Stop(ctx context.Context, handle *DaemonHandle, opts StopOptions) error {
	if _, err := c.requireControl(ctx); err != nil {
		return err
	}
	if handle == nil {
		return invalidRuntimeClient("daemon handle is required")
	}
	return handle.Stop(ctx, opts)
}

func (c *RuntimeAdminClient) Detach(ctx context.Context, handle *DaemonHandle) error {
	if _, err := c.requireControl(ctx); err != nil {
		return err
	}
	if handle == nil {
		return invalidRuntimeClient("daemon handle is required")
	}
	return handle.Detach(ctx)
}

func (c *RuntimeAdminClient) Health(ctx context.Context) (RuntimeHealth, error) {
	if _, err := c.requireControl(ctx); err != nil {
		return RuntimeHealth{}, err
	}
	if c.health == nil {
		return RuntimeHealth{}, invalidRuntimeClient("health client is required")
	}
	return c.health.RuntimeHealth(ctx)
}

func (c *RuntimeAdminClient) Diagnostics(ctx context.Context) (DiagnosticsReport, error) {
	if _, err := c.requireControl(ctx); err != nil {
		return DiagnosticsReport{}, err
	}
	if c.health == nil {
		return DiagnosticsReport{}, invalidRuntimeClient("health client is required")
	}
	return c.health.Diagnostics(ctx)
}

func (c *RuntimeAdminClient) Readiness(ctx context.Context, handle *DaemonHandle) (RuntimeReadiness, error) {
	status, err := c.Status(ctx, handle)
	if err != nil {
		return RuntimeReadiness{}, err
	}
	health, err := c.Health(ctx)
	if err != nil {
		return RuntimeReadiness{}, err
	}
	var diagnostics *DiagnosticsReport
	report, err := c.Diagnostics(ctx)
	if err == nil {
		diagnostics = &report
	}
	messages := append([]string{}, status.Diagnostics...)
	messages = append(messages, health.Diagnostics...)
	return RuntimeReadiness{
		LifecycleState: status.State,
		Endpoints:      status.Endpoints,
		Health:         health,
		Diagnostics:    diagnostics,
		Ready:          daemonRuntimeReady(status.State) && health.Ready(),
		Messages:       messages,
	}, nil
}

func (c *RuntimeAdminClient) requireControl(ctx context.Context) (*DaemonControl, error) {
	if c == nil || c.control == nil {
		return nil, invalidRuntimeClient("runtime admin client is not initialized")
	}
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	return c.control, nil
}
