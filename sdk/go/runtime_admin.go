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
	LifecycleState RuntimeLifecycleState `json:"lifecycle_state"`
	Endpoints      RuntimeHostEndpoints  `json:"endpoints"`
	Health         RuntimeHealth         `json:"health"`
	Diagnostics    *DiagnosticsReport    `json:"diagnostics,omitempty"`
	Ready          bool                  `json:"ready"`
	Messages       []string              `json:"messages,omitempty"`
}

type RuntimeSessionListRequest struct {
	Call              RuntimeCallContext `json:"call"`
	IncludeTerminated *bool              `json:"include_terminated,omitempty"`
}

type RuntimeSession struct {
	Kind                string         `json:"kind,omitempty"`
	SessionID           string         `json:"session_id,omitempty"`
	RuntimeHostURA      string         `json:"runtime_host_ura,omitempty"`
	ControlAuthorityURA string         `json:"control_authority_ura,omitempty"`
	State               string         `json:"state,omitempty"`
	SessionKind         string         `json:"session_kind,omitempty"`
	CreatedUnixMS       int64          `json:"created_unix_ms,omitempty"`
	ExpiresUnixMS       int64          `json:"expires_unix_ms,omitempty"`
	Metadata            map[string]any `json:"metadata,omitempty"`
}

type RuntimeSessionPage struct {
	SystemAbility string           `json:"system_ability,omitempty"`
	State         string           `json:"state,omitempty"`
	Sessions      []RuntimeSession `json:"sessions,omitempty"`
	NextCursor    any              `json:"next_cursor,omitempty"`
	Raw           map[string]any   `json:"raw,omitempty"`
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
	return runtimeSessionPage(output)
}

type RuntimeAdminClient struct {
	lifecycle RuntimeHostLifecycle
	health    *HealthClient
}

// NewRuntimeHostAdminClient creates a provider-neutral runtime administration
// facade over the canonical runtime-host lifecycle.
func NewRuntimeHostAdminClient(lifecycle RuntimeHostLifecycle, health *HealthClient) (*RuntimeAdminClient, error) {
	if lifecycle == nil {
		return nil, invalidRuntimeClient("runtime lifecycle is required")
	}
	return &RuntimeAdminClient{lifecycle: lifecycle, health: health}, nil
}

func (c *RuntimeAdminClient) Discover(ctx context.Context, opts RuntimeHostDiscoverOptions) (RuntimeHostEndpoints, error) {
	control, err := c.requireControl(ctx)
	if err != nil {
		return RuntimeHostEndpoints{}, err
	}
	return control.DiscoverRuntime(ctx, opts)
}

func (c *RuntimeAdminClient) Start(ctx context.Context, request RuntimeHostStartRequest) (*RuntimeHandle, error) {
	control, err := c.requireControl(ctx)
	if err != nil {
		return nil, err
	}
	return control.StartRuntime(ctx, request)
}

func (c *RuntimeAdminClient) Attach(ctx context.Context, opts RuntimeHostAttachOptions) (*RuntimeHandle, error) {
	control, err := c.requireControl(ctx)
	if err != nil {
		return nil, err
	}
	return control.AttachRuntime(ctx, opts)
}

func (c *RuntimeAdminClient) Status(ctx context.Context, handle *RuntimeHandle) (RuntimeHostStatus, error) {
	if _, err := c.requireControl(ctx); err != nil {
		return RuntimeHostStatus{}, err
	}
	if handle == nil {
		return RuntimeHostStatus{}, invalidRuntimeClient("runtime handle is required")
	}
	return handle.Status(ctx)
}

func (c *RuntimeAdminClient) OpenRuntime(ctx context.Context, handle *RuntimeHandle, opts ConnectOptions) (*RuntimeClient, error) {
	if _, err := c.requireControl(ctx); err != nil {
		return nil, err
	}
	if handle == nil {
		return nil, invalidRuntimeClient("runtime handle is required")
	}
	return handle.OpenRuntime(ctx, opts)
}

func (c *RuntimeAdminClient) Stop(ctx context.Context, handle *RuntimeHandle, opts RuntimeHostStopOptions) error {
	if _, err := c.requireControl(ctx); err != nil {
		return err
	}
	if handle == nil {
		return invalidRuntimeClient("runtime handle is required")
	}
	return handle.StopRuntime(ctx, opts)
}

func (c *RuntimeAdminClient) Detach(ctx context.Context, handle *RuntimeHandle) error {
	if _, err := c.requireControl(ctx); err != nil {
		return err
	}
	if handle == nil {
		return invalidRuntimeClient("runtime handle is required")
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

func (c *RuntimeAdminClient) Readiness(ctx context.Context, handle *RuntimeHandle) (RuntimeReadiness, error) {
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
		Ready:          runtimeLifecycleReady(status.State) && health.Ready(),
		Messages:       messages,
	}, nil
}

func (c *RuntimeAdminClient) requireControl(ctx context.Context) (RuntimeHostLifecycle, error) {
	if c == nil || c.lifecycle == nil {
		return nil, invalidRuntimeClient("runtime admin client is not initialized")
	}
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	return c.lifecycle, nil
}

func runtimeAdminCall(call RuntimeCallContext, ability string) RuntimeCallContext {
	metadata := cloneRuntimeAdminMap(call.Metadata)
	metadata["sdk_profile"] = runtimeAdminProfile
	metadata["system_ability"] = ability
	call.Metadata = metadata
	return call
}

func runtimeSessionPage(output map[string]any) (RuntimeSessionPage, error) {
	rows, err := requiredRuntimeAdminSessionRows(output, "sessions")
	if err != nil {
		return RuntimeSessionPage{}, err
	}
	sessions := make([]RuntimeSession, 0, len(rows))
	for _, row := range rows {
		if _, ok := row["device_ura"]; ok {
			return RuntimeSessionPage{}, invalidRuntimePayload("runtime admin session row preserves retired device_ura field", nil)
		}
		if _, ok := row["authority_ura"]; ok {
			return RuntimeSessionPage{}, invalidRuntimePayload("runtime admin session row preserves retired authority_ura field", nil)
		}
		sessionID, err := requiredRuntimeAdminString(row, "session_id")
		if err != nil {
			return RuntimeSessionPage{}, err
		}
		runtimeHostURA, err := requiredRuntimeAdminString(row, "runtime_host_ura")
		if err != nil {
			return RuntimeSessionPage{}, err
		}
		controlAuthorityURA, err := requiredRuntimeAdminString(row, "control_authority_ura")
		if err != nil {
			return RuntimeSessionPage{}, err
		}
		state, err := requiredRuntimeAdminString(row, "state")
		if err != nil {
			return RuntimeSessionPage{}, err
		}
		sessions = append(sessions, RuntimeSession{
			Kind:                runtimeAdminString(row, "kind"),
			SessionID:           sessionID,
			RuntimeHostURA:      runtimeHostURA,
			ControlAuthorityURA: controlAuthorityURA,
			State:               state,
			SessionKind:         runtimeAdminString(row, "session_kind"),
			CreatedUnixMS:       runtimeAdminInt64(row["created_unix_ms"]),
			ExpiresUnixMS:       runtimeAdminInt64(row["expires_unix_ms"]),
			Metadata:            runtimeAdminMap(row["metadata"]),
		})
	}
	return RuntimeSessionPage{
		SystemAbility: runtimeAdminSessionListAbility,
		State:         runtimeAdminString(output, "state"),
		Sessions:      sessions,
		NextCursor:    output["next_cursor"],
		Raw:           cloneRuntimeAdminMap(output),
	}, nil
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

func requiredRuntimeAdminString(value map[string]any, key string) (string, error) {
	if raw, ok := value[key].(string); ok && strings.TrimSpace(raw) != "" {
		return strings.TrimSpace(raw), nil
	}
	return "", invalidRuntimePayload("runtime admin session row field "+key+" is required", nil)
}

func requiredRuntimeAdminBool(output map[string]any, field string) (bool, error) {
	raw, ok := output[field].(bool)
	if !ok {
		return false, invalidRuntimePayload("runtime admin response field "+field+" must be a boolean", nil)
	}
	return raw, nil
}

func optionalRuntimeAdminBool(output map[string]any, field string) (bool, error) {
	value, exists := output[field]
	if !exists || value == nil {
		return false, nil
	}
	if raw, ok := value.(bool); ok {
		return raw, nil
	}
	return false, invalidRuntimePayload("runtime admin response field "+field+" must be a boolean", nil)
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

func requiredRuntimeAdminSessionRows(output map[string]any, field string) ([]map[string]any, error) {
	value, exists := output[field]
	if !exists {
		return nil, invalidRuntimePayload("runtime admin response field "+field+" must be an array", nil)
	}
	if typed, ok := value.([]map[string]any); ok {
		return typed, nil
	}
	rows, ok := value.([]any)
	if !ok {
		return nil, invalidRuntimePayload("runtime admin response field "+field+" must be an array", nil)
	}
	result := make([]map[string]any, 0, len(rows))
	for _, row := range rows {
		typed, ok := row.(map[string]any)
		if !ok {
			return nil, invalidRuntimePayload("runtime admin response field "+field+" entries must be objects", nil)
		}
		result = append(result, typed)
	}
	return result, nil
}
