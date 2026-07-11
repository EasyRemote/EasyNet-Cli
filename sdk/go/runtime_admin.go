package easynet

import (
	"context"
	"strings"
)

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

const (
	runtimeAdminProfile             = "runtime_admin"
	runtimeAdminSessionListAbility  = "session.list"
	runtimeAdminDeviceRevokeAbility = "federation.revoke"
)

type RuntimeSessionListRequest struct {
	Call              RuntimeCallContext `json:"call"`
	IncludeTerminated *bool              `json:"include_terminated,omitempty"`
}

type RuntimeSession struct {
	Kind          string         `json:"kind,omitempty"`
	SessionID     string         `json:"session_id,omitempty"`
	DeviceURA     string         `json:"device_ura,omitempty"`
	HubURA        string         `json:"hub_ura,omitempty"`
	State         string         `json:"state,omitempty"`
	SessionKind   string         `json:"session_kind,omitempty"`
	CreatedUnixMS int64          `json:"created_unix_ms,omitempty"`
	ExpiresUnixMS int64          `json:"expires_unix_ms,omitempty"`
	Metadata      map[string]any `json:"metadata,omitempty"`
}

type RuntimeSessionPage struct {
	SystemAbility string           `json:"system_ability,omitempty"`
	State         string           `json:"state,omitempty"`
	Sessions      []RuntimeSession `json:"sessions,omitempty"`
	NextCursor    any              `json:"next_cursor,omitempty"`
	Raw           map[string]any   `json:"raw,omitempty"`
}

type RuntimeDeviceRevokeRequest struct {
	Call      RuntimeCallContext `json:"call"`
	DeviceURA string             `json:"device_ura"`
	Reason    string             `json:"reason"`
}

type RuntimeDeviceRevokeResult struct {
	SystemAbility          string         `json:"system_ability,omitempty"`
	DeviceURA              string         `json:"device_ura,omitempty"`
	Ack                    bool           `json:"ack"`
	RuntimeNotReady        bool           `json:"runtime_not_ready,omitempty"`
	RuntimeCatalogNotReady bool           `json:"runtime_catalog_not_ready,omitempty"`
	Raw                    map[string]any `json:"raw,omitempty"`
}

type RuntimeAdminAbilityClient struct {
	ability *RuntimeAbilityClient
}

func NewRuntimeAdminAbilityClient(ability *RuntimeAbilityClient) (*RuntimeAdminAbilityClient, error) {
	if ability == nil {
		return nil, invalidRuntimeClient("runtime ability client is required")
	}
	return &RuntimeAdminAbilityClient{ability: ability}, nil
}

func (c *RuntimeAdminAbilityClient) ListSessions(ctx context.Context, request RuntimeSessionListRequest) (RuntimeSessionPage, error) {
	if c == nil || c.ability == nil {
		return RuntimeSessionPage{}, invalidRuntimeClient("runtime admin ability client is not initialized")
	}
	args := map[string]any{}
	if request.IncludeTerminated != nil {
		args["include_terminated"] = *request.IncludeTerminated
	}
	output, err := c.ability.Invoke(ctx, runtimeAdminCall(request.Call, runtimeAdminSessionListAbility), runtimeAdminSessionListAbility, args)
	if err != nil {
		return RuntimeSessionPage{}, err
	}
	return runtimeSessionPage(output), nil
}

func (c *RuntimeAdminAbilityClient) RevokeDevice(ctx context.Context, request RuntimeDeviceRevokeRequest) (RuntimeDeviceRevokeResult, error) {
	if c == nil || c.ability == nil {
		return RuntimeDeviceRevokeResult{}, invalidRuntimeClient("runtime admin ability client is not initialized")
	}
	deviceURA := strings.TrimSpace(request.DeviceURA)
	reason := strings.TrimSpace(request.Reason)
	if deviceURA == "" || reason == "" {
		return RuntimeDeviceRevokeResult{}, invalidRuntimePayload("device_ura and reason are required", nil)
	}
	output, err := c.ability.Invoke(ctx, runtimeAdminCall(request.Call, runtimeAdminDeviceRevokeAbility), runtimeAdminDeviceRevokeAbility, map[string]any{
		"agent_ura": deviceURA,
		"reason":    reason,
	})
	if err != nil {
		return RuntimeDeviceRevokeResult{}, err
	}
	ack := runtimeAdminBool(output["ack"], true)
	return RuntimeDeviceRevokeResult{
		SystemAbility:          runtimeAdminDeviceRevokeAbility,
		DeviceURA:              deviceURA,
		Ack:                    ack,
		RuntimeNotReady:        runtimeAdminBool(output["runtime_not_ready"], false),
		RuntimeCatalogNotReady: runtimeAdminBool(output["runtime_catalog_not_ready"], false),
		Raw:                    cloneRuntimeAdminMap(output),
	}, nil
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

func runtimeAdminCall(call RuntimeCallContext, ability string) RuntimeCallContext {
	metadata := cloneRuntimeAdminMap(call.Metadata)
	metadata["sdk_profile"] = runtimeAdminProfile
	metadata["system_ability"] = ability
	call.Metadata = metadata
	return call
}

func runtimeSessionPage(output map[string]any) RuntimeSessionPage {
	rows := runtimeAdminMapSlice(output["sessions"])
	if len(rows) == 0 {
		rows = runtimeAdminMapSlice(output["items"])
	}
	sessions := make([]RuntimeSession, 0, len(rows))
	for _, row := range rows {
		sessions = append(sessions, RuntimeSession{
			Kind:          runtimeAdminString(row, "kind"),
			SessionID:     runtimeAdminString(row, "session_id"),
			DeviceURA:     runtimeAdminString(row, "device_ura"),
			HubURA:        runtimeAdminString(row, "hub_ura"),
			State:         runtimeAdminString(row, "state"),
			SessionKind:   runtimeAdminString(row, "session_kind"),
			CreatedUnixMS: runtimeAdminInt64(row["created_unix_ms"]),
			ExpiresUnixMS: runtimeAdminInt64(row["expires_unix_ms"]),
			Metadata:      runtimeAdminMap(row["metadata"]),
		})
	}
	return RuntimeSessionPage{
		SystemAbility: runtimeAdminSessionListAbility,
		State:         runtimeAdminString(output, "state"),
		Sessions:      sessions,
		NextCursor:    output["next_cursor"],
		Raw:           cloneRuntimeAdminMap(output),
	}
}

func cloneRuntimeAdminMap(input map[string]any) map[string]any {
	output := make(map[string]any, len(input)+2)
	for key, value := range input {
		output[key] = value
	}
	return output
}

func runtimeAdminString(value map[string]any, keys ...string) string {
	for _, key := range keys {
		if raw, ok := value[key].(string); ok && strings.TrimSpace(raw) != "" {
			return strings.TrimSpace(raw)
		}
	}
	return ""
}

func runtimeAdminBool(value any, fallback bool) bool {
	if raw, ok := value.(bool); ok {
		return raw
	}
	return fallback
}

func runtimeAdminInt64(value any) int64 {
	switch raw := value.(type) {
	case float64:
		return int64(raw)
	case int64:
		return raw
	case int:
		return int64(raw)
	default:
		return 0
	}
}

func runtimeAdminMap(value any) map[string]any {
	if raw, ok := value.(map[string]any); ok && raw != nil {
		return raw
	}
	return map[string]any{}
}

func runtimeAdminMapSlice(value any) []map[string]any {
	rows, ok := value.([]any)
	if !ok {
		if typed, ok := value.([]map[string]any); ok {
			return typed
		}
		return []map[string]any{}
	}
	result := make([]map[string]any, 0, len(rows))
	for _, row := range rows {
		if typed, ok := row.(map[string]any); ok {
			result = append(result, typed)
		}
	}
	return result
}
