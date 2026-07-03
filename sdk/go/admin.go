package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

const adminGatewayProfile = "admin_gateway"

// AdminCarrierBase is the complete carrier context shared by Admin + Gateway operations.
type AdminCarrierBase struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

// AdminGatewayStatusRequest asks the daemon for gateway readiness facts.
type AdminGatewayStatusRequest struct {
	RequirePublicListener *bool          `json:"require_public_listener,omitempty"`
	Metadata              map[string]any `json:"metadata,omitempty"`
}

// AdminAgentListRequest requests hosted agent lifecycle records.
type AdminAgentListRequest struct {
	AdminCarrierBase
}

// AdminAgentStartRequest starts or registers a hosted agent through daemon policy.
type AdminAgentStartRequest struct {
	AdminCarrierBase
	Name                 string         `json:"name"`
	AgentType            string         `json:"agent_type,omitempty"`
	Entry                map[string]any `json:"entry,omitempty"`
	Model                string         `json:"model,omitempty"`
	Label                string         `json:"label,omitempty"`
	Command              string         `json:"command,omitempty"`
	CommandArgs          []string       `json:"command_args,omitempty"`
	RootPath             string         `json:"root_path,omitempty"`
	ModelPresent         *bool          `json:"model_present,omitempty"`
	MaterializeDirectory *bool          `json:"materialize_directory,omitempty"`
	UpdateExistingSpec   *bool          `json:"update_existing_spec,omitempty"`
	ProjectWorkspace     *bool          `json:"project_workspace,omitempty"`
}

// AdminAgentStopRequest stops a hosted agent by owner-local name or Agent URA.
type AdminAgentStopRequest struct {
	AdminCarrierBase
	Name     string `json:"name,omitempty"`
	AgentURA string `json:"agent_ura,omitempty"`
}

// AdminAgentRefreshRequest refreshes hosted agent runtime registration.
type AdminAgentRefreshRequest struct {
	AdminCarrierBase
	Name string `json:"name,omitempty"`
}

// AdminSessionListRequest requests daemon device-session records.
type AdminSessionListRequest struct {
	AdminCarrierBase
	IncludeTerminated *bool `json:"include_terminated,omitempty"`
}

type GatewayListener struct {
	Kind     string `json:"kind"`
	Endpoint string `json:"endpoint"`
	Ready    bool   `json:"ready"`
	Public   bool   `json:"public"`
}

// GatewayStatus preserves daemon readiness flags without collapsing degraded states.
type GatewayStatus struct {
	Profile             string            `json:"profile"`
	GatewayID           string            `json:"gateway_id"`
	Ready               bool              `json:"ready"`
	State               string            `json:"state"`
	ProcessLive         bool              `json:"process_live"`
	ControlReady        bool              `json:"control_ready"`
	RuntimeReady        bool              `json:"runtime_ready"`
	DirectoryReady      bool              `json:"directory_ready"`
	TrustReady          bool              `json:"trust_ready"`
	PublicListenerReady bool              `json:"public_listener_ready"`
	Listeners           []GatewayListener `json:"listeners"`
	Identity            map[string]any    `json:"identity"`
	Metadata            map[string]any    `json:"metadata"`
}

// AdminAgentPage is the Admin + Gateway projection of daemon agent records.
type AdminAgentPage struct {
	Profile    string         `json:"profile"`
	Kind       string         `json:"kind"`
	State      string         `json:"state"`
	Items      []AgentRecord  `json:"items"`
	NextCursor any            `json:"next_cursor"`
	Metadata   map[string]any `json:"metadata"`
}

// AdminGatewayResult is the generic daemon admin lifecycle/status projection.
type AdminGatewayResult struct {
	Profile                string           `json:"profile"`
	Kind                   string           `json:"kind"`
	Operation              string           `json:"operation,omitempty"`
	State                  string           `json:"state"`
	AgentURA               *string          `json:"agent_ura"`
	DeviceURA              *string          `json:"device_ura"`
	Ack                    *bool            `json:"ack"`
	RuntimeNotReady        bool             `json:"runtime_not_ready"`
	RuntimeCatalogNotReady bool             `json:"runtime_catalog_not_ready"`
	Items                  []map[string]any `json:"items,omitempty"`
	NextCursor             any              `json:"next_cursor,omitempty"`
	Metadata               map[string]any   `json:"metadata"`
}

type AgentStartResult = AdminGatewayResult
type AgentStopResult = AdminGatewayResult
type AgentRefreshResult = AdminGatewayResult
type DeviceSessionPage = AdminGatewayResult

// AdminTransport supplies daemon Admin + Gateway operations behind the facade.
type AdminTransport interface {
	BuildAgentListInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildAgentStartInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildAgentStopInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildAgentRefreshInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildSessionListInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	GatewayStatus(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListAgents(ctx context.Context, requestJSON []byte) ([]byte, error)
	AgentStart(ctx context.Context, requestJSON []byte) ([]byte, error)
	AgentStop(ctx context.Context, requestJSON []byte) ([]byte, error)
	AgentRefresh(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListDeviceSessions(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// AdminTransportFunc adapts functions into an AdminTransport.
type AdminTransportFunc struct {
	BuildAgentListInvocationFunc    func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildAgentStartInvocationFunc   func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildAgentStopInvocationFunc    func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildAgentRefreshInvocationFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildSessionListInvocationFunc  func(ctx context.Context, requestJSON []byte) ([]byte, error)
	GatewayStatusFunc               func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListAgentsFunc                  func(ctx context.Context, requestJSON []byte) ([]byte, error)
	AgentStartFunc                  func(ctx context.Context, requestJSON []byte) ([]byte, error)
	AgentStopFunc                   func(ctx context.Context, requestJSON []byte) ([]byte, error)
	AgentRefreshFunc                func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListDeviceSessionsFunc          func(ctx context.Context, requestJSON []byte) ([]byte, error)
}

func (f AdminTransportFunc) BuildAgentListInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildAgentListInvocationFunc == nil {
		return nil, invalidRuntimeClient("admin agent-list invocation transport function is required")
	}
	return f.BuildAgentListInvocationFunc(ctx, requestJSON)
}

func (f AdminTransportFunc) BuildAgentStartInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildAgentStartInvocationFunc == nil {
		return nil, invalidRuntimeClient("admin agent-start invocation transport function is required")
	}
	return f.BuildAgentStartInvocationFunc(ctx, requestJSON)
}

func (f AdminTransportFunc) BuildAgentStopInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildAgentStopInvocationFunc == nil {
		return nil, invalidRuntimeClient("admin agent-stop invocation transport function is required")
	}
	return f.BuildAgentStopInvocationFunc(ctx, requestJSON)
}

func (f AdminTransportFunc) BuildAgentRefreshInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildAgentRefreshInvocationFunc == nil {
		return nil, invalidRuntimeClient("admin agent-refresh invocation transport function is required")
	}
	return f.BuildAgentRefreshInvocationFunc(ctx, requestJSON)
}

func (f AdminTransportFunc) BuildSessionListInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildSessionListInvocationFunc == nil {
		return nil, invalidRuntimeClient("admin session-list invocation transport function is required")
	}
	return f.BuildSessionListInvocationFunc(ctx, requestJSON)
}

func (f AdminTransportFunc) GatewayStatus(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.GatewayStatusFunc == nil {
		return nil, invalidRuntimeClient("admin gateway-status transport function is required")
	}
	return f.GatewayStatusFunc(ctx, requestJSON)
}

func (f AdminTransportFunc) ListAgents(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ListAgentsFunc == nil {
		return nil, invalidRuntimeClient("admin list-agents transport function is required")
	}
	return f.ListAgentsFunc(ctx, requestJSON)
}

func (f AdminTransportFunc) AgentStart(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.AgentStartFunc == nil {
		return nil, invalidRuntimeClient("admin agent-start transport function is required")
	}
	return f.AgentStartFunc(ctx, requestJSON)
}

func (f AdminTransportFunc) AgentStop(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.AgentStopFunc == nil {
		return nil, invalidRuntimeClient("admin agent-stop transport function is required")
	}
	return f.AgentStopFunc(ctx, requestJSON)
}

func (f AdminTransportFunc) AgentRefresh(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.AgentRefreshFunc == nil {
		return nil, invalidRuntimeClient("admin agent-refresh transport function is required")
	}
	return f.AgentRefreshFunc(ctx, requestJSON)
}

func (f AdminTransportFunc) ListDeviceSessions(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ListDeviceSessionsFunc == nil {
		return nil, invalidRuntimeClient("admin list-device-sessions transport function is required")
	}
	return f.ListDeviceSessionsFunc(ctx, requestJSON)
}

// AdminClient is the Admin + Gateway profile facade.
type AdminClient struct {
	transport AdminTransport
}

func NewAdminClient(transport AdminTransport) (*AdminClient, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("admin transport is required")
	}
	return &AdminClient{transport: transport}, nil
}

func (c *AdminClient) BuildAgentListInvocation(ctx context.Context, req AdminAgentListRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateAdminAgentListRequest, c.transport.BuildAgentListInvocation, "admin agent-list invocation failed")
}

func (c *AdminClient) BuildAgentStartInvocation(ctx context.Context, req AdminAgentStartRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateAdminAgentStartRequest, c.transport.BuildAgentStartInvocation, "admin agent-start invocation failed")
}

func (c *AdminClient) BuildAgentStopInvocation(ctx context.Context, req AdminAgentStopRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateAdminAgentStopRequest, c.transport.BuildAgentStopInvocation, "admin agent-stop invocation failed")
}

func (c *AdminClient) BuildAgentRefreshInvocation(ctx context.Context, req AdminAgentRefreshRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateAdminAgentRefreshRequest, c.transport.BuildAgentRefreshInvocation, "admin agent-refresh invocation failed")
}

func (c *AdminClient) BuildSessionListInvocation(ctx context.Context, req AdminSessionListRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateAdminSessionListRequest, c.transport.BuildSessionListInvocation, "admin session-list invocation failed")
}

func (c *AdminClient) GatewayStatus(ctx context.Context, req AdminGatewayStatusRequest) (GatewayStatus, error) {
	if err := c.requireReady(ctx); err != nil {
		return GatewayStatus{}, err
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return GatewayStatus{}, invalidRuntimePayload(fmt.Sprintf("encode gateway status request: %v", err), err)
	}
	raw, err := c.transport.GatewayStatus(ctx, requestJSON)
	if err != nil {
		return GatewayStatus{}, wrapAdminTransportError("admin gateway status failed", err)
	}
	return NewGatewayStatusFromJSON(raw)
}

func (c *AdminClient) ListAgents(ctx context.Context, req AdminAgentListRequest) (AdminAgentPage, error) {
	if err := c.requireReady(ctx); err != nil {
		return AdminAgentPage{}, err
	}
	requestJSON, err := marshalAdminRequest(req, validateAdminAgentListRequest)
	if err != nil {
		return AdminAgentPage{}, err
	}
	raw, err := c.transport.ListAgents(ctx, requestJSON)
	if err != nil {
		return AdminAgentPage{}, wrapAdminTransportError("admin list agents failed", err)
	}
	return NewAdminAgentPageFromJSON(raw)
}

func (c *AdminClient) AgentStart(ctx context.Context, req AdminAgentStartRequest) (AgentStartResult, error) {
	return c.resultOperation(ctx, req, validateAdminAgentStartRequest, c.transport.AgentStart, "admin agent start failed")
}

func (c *AdminClient) AgentStop(ctx context.Context, req AdminAgentStopRequest) (AgentStopResult, error) {
	return c.resultOperation(ctx, req, validateAdminAgentStopRequest, c.transport.AgentStop, "admin agent stop failed")
}

func (c *AdminClient) AgentRefresh(ctx context.Context, req AdminAgentRefreshRequest) (AgentRefreshResult, error) {
	return c.resultOperation(ctx, req, validateAdminAgentRefreshRequest, c.transport.AgentRefresh, "admin agent refresh failed")
}

func (c *AdminClient) ListDeviceSessions(ctx context.Context, req AdminSessionListRequest) (DeviceSessionPage, error) {
	return c.resultOperation(ctx, req, validateAdminSessionListRequest, c.transport.ListDeviceSessions, "admin list device sessions failed")
}

func (c *AdminClient) buildInvocation(ctx context.Context, req any, validate func(any) error, fn func(context.Context, []byte) ([]byte, error), label string) (InvocationDraft, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationDraft{}, err
	}
	requestJSON, err := marshalAdminRequest(req, validate)
	if err != nil {
		return InvocationDraft{}, err
	}
	raw, err := fn(ctx, requestJSON)
	if err != nil {
		return InvocationDraft{}, wrapAdminTransportError(label, err)
	}
	return NewInvocationDraftFromJSON(raw)
}

func (c *AdminClient) resultOperation(ctx context.Context, req any, validate func(any) error, fn func(context.Context, []byte) ([]byte, error), label string) (AdminGatewayResult, error) {
	if err := c.requireReady(ctx); err != nil {
		return AdminGatewayResult{}, err
	}
	requestJSON, err := marshalAdminRequest(req, validate)
	if err != nil {
		return AdminGatewayResult{}, err
	}
	raw, err := fn(ctx, requestJSON)
	if err != nil {
		return AdminGatewayResult{}, wrapAdminTransportError(label, err)
	}
	return NewAdminGatewayResultFromJSON(raw)
}

func (c *AdminClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("admin client is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	return nil
}

func NewGatewayStatusFromJSON(raw []byte) (GatewayStatus, error) {
	var status GatewayStatus
	if err := json.Unmarshal(raw, &status); err != nil {
		return GatewayStatus{}, invalidRuntimePayload(fmt.Sprintf("decode gateway status JSON: %v", err), err)
	}
	if status.Profile != adminGatewayProfile || status.GatewayID == "" || status.State == "" ||
		status.Listeners == nil || status.Metadata == nil {
		return GatewayStatus{}, invalidRuntimePayload("invalid gateway status projection", nil)
	}
	for _, listener := range status.Listeners {
		if listener.Kind == "" || listener.Endpoint == "" {
			return GatewayStatus{}, invalidRuntimePayload("invalid gateway listener projection", nil)
		}
	}
	return status, nil
}

func NewAdminAgentPageFromJSON(raw []byte) (AdminAgentPage, error) {
	var page AdminAgentPage
	if err := json.Unmarshal(raw, &page); err != nil {
		return AdminAgentPage{}, invalidRuntimePayload(fmt.Sprintf("decode admin agent page JSON: %v", err), err)
	}
	if page.Profile != adminGatewayProfile || page.Kind != "agent_records" || page.State == "" ||
		page.Items == nil || page.Metadata == nil {
		return AdminAgentPage{}, invalidRuntimePayload("invalid admin agent page projection", nil)
	}
	for _, item := range page.Items {
		if item.Name == "" || item.State == "" || item.Runtime == "" || item.Metadata == nil {
			return AdminAgentPage{}, invalidRuntimePayload("invalid admin agent record projection", nil)
		}
	}
	return page, nil
}

func NewAdminGatewayResultFromJSON(raw []byte) (AdminGatewayResult, error) {
	var result AdminGatewayResult
	if err := json.Unmarshal(raw, &result); err != nil {
		return AdminGatewayResult{}, invalidRuntimePayload(fmt.Sprintf("decode admin result JSON: %v", err), err)
	}
	if result.Profile != adminGatewayProfile || result.Kind == "" || result.State == "" || result.Metadata == nil {
		return AdminGatewayResult{}, invalidRuntimePayload("invalid admin result projection", nil)
	}
	return result, nil
}

func marshalAdminRequest(req any, validate func(any) error) ([]byte, error) {
	if err := validate(req); err != nil {
		return nil, err
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode admin request: %v", err), err)
	}
	return requestJSON, nil
}

func validateAdminAgentListRequest(req any) error {
	return validateAdminCarrierBase(req.(AdminAgentListRequest).AdminCarrierBase)
}

func validateAdminAgentStartRequest(req any) error {
	value := req.(AdminAgentStartRequest)
	if err := validateAdminCarrierBase(value.AdminCarrierBase); err != nil {
		return err
	}
	if err := validateAdminAgentName(value.Name, "name"); err != nil {
		return err
	}
	if value.AgentType == "" && value.Entry == nil {
		return invalidRuntimePayload("either agent_type or entry is required", nil)
	}
	if value.RootPath != "" && !strings.HasPrefix(value.RootPath, "/") {
		return invalidRuntimePayload("root_path must be absolute", nil)
	}
	if strings.Contains(value.RootPath, "/../") || strings.HasSuffix(value.RootPath, "/..") {
		return invalidRuntimePayload("root_path must not contain parent traversal", nil)
	}
	return nil
}

func validateAdminAgentStopRequest(req any) error {
	value := req.(AdminAgentStopRequest)
	if err := validateAdminCarrierBase(value.AdminCarrierBase); err != nil {
		return err
	}
	if value.Name == "" && value.AgentURA == "" {
		return invalidRuntimePayload("either name or agent_ura is required", nil)
	}
	if value.Name != "" {
		if err := validateAdminAgentName(value.Name, "name"); err != nil {
			return err
		}
	}
	if value.AgentURA != "" {
		if err := validateHostedAgentURA(value.AgentURA); err != nil {
			return err
		}
		if value.Name != "" && !strings.HasSuffix(value.AgentURA, "."+value.Name) {
			return invalidRuntimePayload("agent_ura must name the same hosted agent as name", nil)
		}
	}
	return nil
}

func validateAdminAgentRefreshRequest(req any) error {
	value := req.(AdminAgentRefreshRequest)
	if err := validateAdminCarrierBase(value.AdminCarrierBase); err != nil {
		return err
	}
	if value.Name != "" {
		return validateAdminAgentName(value.Name, "name")
	}
	return nil
}

func validateAdminSessionListRequest(req any) error {
	return validateAdminCarrierBase(req.(AdminSessionListRequest).AdminCarrierBase)
}

func validateAdminCarrierBase(base AdminCarrierBase) error {
	if base.CallerURA == "" || base.CalleeURA == "" || base.SubjectURA == "" ||
		base.DescriptorVersion == "" || base.NonceBase64 == "" || base.CausalContext == nil {
		return invalidRuntimePayload("complete admin invocation carrier is required", nil)
	}
	return nil
}

func validateAdminAgentName(value string, field string) error {
	if strings.TrimSpace(value) == "" {
		return invalidRuntimePayload(field+" must not be empty", nil)
	}
	if value == "device" || strings.HasPrefix(value, "device.") {
		return invalidRuntimePayload("device system agents are not managed by hosted agent lifecycle", nil)
	}
	if strings.Contains(value, "/") || strings.Contains(value, "\\") || strings.ContainsAny(value, " \t\r\n") {
		return invalidRuntimePayload(field+" must be an owner-local agent id", nil)
	}
	return nil
}

func validateHostedAgentURA(value string) error {
	if !strings.Contains(value, "/agent/") {
		return invalidRuntimePayload("agent_ura must be an Agent URA", nil)
	}
	if strings.Contains(value, "/agent/device.") {
		return invalidRuntimePayload("device-sponsored System Agents are not managed by hosted agent lifecycle", nil)
	}
	return nil
}

func wrapAdminTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
