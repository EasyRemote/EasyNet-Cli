package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
)

const (
	DefaultDirectoryPageSize = 50
	MaxDirectoryPageSize     = 500
)

const directoryIdentityProfile = "directory_identity"
const directoryReadModelSource = "read_model"

// DirectoryQueryBase is the complete carrier context for directory read-model requests.
type DirectoryQueryBase struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	Limit             int            `json:"limit,omitempty"`
	Cursor            string         `json:"cursor,omitempty"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

// ResolveQuery asks the daemon-owned directory resolver for a stable projection.
type ResolveQuery struct {
	DirectoryQueryBase
	QueryName   string `json:"query_name,omitempty"`
	AbilityName string `json:"ability_name,omitempty"`
	QType       string `json:"qtype,omitempty"`
	RealmHint   string `json:"realm_hint,omitempty"`
}

// DeviceQuery requests a paginated device read model page.
type DeviceQuery struct {
	DirectoryQueryBase
}

// AgentQuery requests a paginated agent read model page.
type AgentQuery struct {
	DirectoryQueryBase
}

// AbilityQuery requests a paginated ability read model page.
type AbilityQuery struct {
	DirectoryQueryBase
	Scope      string `json:"scope,omitempty"`
	OwnerURA   string `json:"owner_ura,omitempty"`
	AbilityURA string `json:"ability_ura,omitempty"`
}

// DirectoryTransport supplies read-model operations behind the SDK facade.
type DirectoryTransport interface {
	Resolve(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListDevices(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListAgents(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListAbilities(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// DirectoryTransportFunc adapts functions into a DirectoryTransport.
type DirectoryTransportFunc struct {
	ResolveFunc       func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListDevicesFunc   func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListAgentsFunc    func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListAbilitiesFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
}

func (f DirectoryTransportFunc) Resolve(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ResolveFunc == nil {
		return nil, invalidRuntimeClient("directory resolve transport function is required")
	}
	return f.ResolveFunc(ctx, requestJSON)
}

func (f DirectoryTransportFunc) ListDevices(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ListDevicesFunc == nil {
		return nil, invalidRuntimeClient("directory list-devices transport function is required")
	}
	return f.ListDevicesFunc(ctx, requestJSON)
}

func (f DirectoryTransportFunc) ListAgents(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ListAgentsFunc == nil {
		return nil, invalidRuntimeClient("directory list-agents transport function is required")
	}
	return f.ListAgentsFunc(ctx, requestJSON)
}

func (f DirectoryTransportFunc) ListAbilities(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ListAbilitiesFunc == nil {
		return nil, invalidRuntimeClient("directory list-abilities transport function is required")
	}
	return f.ListAbilitiesFunc(ctx, requestJSON)
}

// DirectoryClient is the Directory + Identity read-model facade.
type DirectoryClient struct {
	transport DirectoryTransport
}

func NewDirectoryClient(transport DirectoryTransport) (*DirectoryClient, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("directory transport is required")
	}
	return &DirectoryClient{transport: transport}, nil
}

func (c *DirectoryClient) Resolve(ctx context.Context, query ResolveQuery) (ResolvedRef, error) {
	if err := c.requireReady(ctx); err != nil {
		return ResolvedRef{}, err
	}
	if err := validateDirectoryQueryBase(query.DirectoryQueryBase, false); err != nil {
		return ResolvedRef{}, err
	}
	if query.QueryName == "" && query.RealmHint == "" {
		return ResolvedRef{}, invalidRuntimePayload("query_name or realm_hint is required", nil)
	}
	requestJSON, err := json.Marshal(query)
	if err != nil {
		return ResolvedRef{}, invalidRuntimePayload(fmt.Sprintf("encode directory resolve request: %v", err), err)
	}
	raw, err := c.transport.Resolve(ctx, requestJSON)
	if err != nil {
		return ResolvedRef{}, wrapDirectoryTransportError("directory resolve failed", err)
	}
	return NewResolvedRefFromJSON(raw)
}

func (c *DirectoryClient) ListDevices(ctx context.Context, query DeviceQuery) (DevicePage, error) {
	if err := c.requireReady(ctx); err != nil {
		return DevicePage{}, err
	}
	requestJSON, err := marshalDirectoryPageQuery(query.DirectoryQueryBase)
	if err != nil {
		return DevicePage{}, err
	}
	raw, err := c.transport.ListDevices(ctx, requestJSON)
	if err != nil {
		return DevicePage{}, wrapDirectoryTransportError("directory list devices failed", err)
	}
	return NewDevicePageFromJSON(raw)
}

func (c *DirectoryClient) ListAgents(ctx context.Context, query AgentQuery) (AgentPage, error) {
	if err := c.requireReady(ctx); err != nil {
		return AgentPage{}, err
	}
	requestJSON, err := marshalDirectoryPageQuery(query.DirectoryQueryBase)
	if err != nil {
		return AgentPage{}, err
	}
	raw, err := c.transport.ListAgents(ctx, requestJSON)
	if err != nil {
		return AgentPage{}, wrapDirectoryTransportError("directory list agents failed", err)
	}
	return NewAgentPageFromJSON(raw)
}

func (c *DirectoryClient) ListAbilities(ctx context.Context, query AbilityQuery) (AbilityPage, error) {
	if err := c.requireReady(ctx); err != nil {
		return AbilityPage{}, err
	}
	query.DirectoryQueryBase = normalizeDirectoryPageQuery(query.DirectoryQueryBase)
	if err := validateDirectoryQueryBase(query.DirectoryQueryBase, true); err != nil {
		return AbilityPage{}, err
	}
	requestJSON, err := json.Marshal(query)
	if err != nil {
		return AbilityPage{}, invalidRuntimePayload(fmt.Sprintf("encode directory ability query: %v", err), err)
	}
	raw, err := c.transport.ListAbilities(ctx, requestJSON)
	if err != nil {
		return AbilityPage{}, wrapDirectoryTransportError("directory list abilities failed", err)
	}
	return NewAbilityPageFromJSON(raw)
}

func (c *DirectoryClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("directory client is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	return nil
}

// ResolvedRef is the daemon/Axon-owned directory resolution projection.
type ResolvedRef struct {
	Profile         string           `json:"profile"`
	Kind            string           `json:"kind"`
	AnswerKind      string           `json:"answer_kind"`
	QueryName       *string          `json:"query_name"`
	CanonicalName   *string          `json:"canonical_name"`
	OwnerURA        *string          `json:"owner_ura"`
	AbilityURA      *string          `json:"ability_ura"`
	RouteURA        *string          `json:"route_ura"`
	NextHop         map[string]any   `json:"next_hop"`
	SelectedRoute   map[string]any   `json:"selected_route"`
	RouteCandidates []map[string]any `json:"route_candidates"`
	Records         []map[string]any `json:"records"`
	Negative        map[string]any   `json:"negative"`
	ReleaseProfile  *string          `json:"release_profile"`
	Authority       map[string]any   `json:"authority"`
	CachePolicy     map[string]any   `json:"cache_policy"`
	Metadata        map[string]any   `json:"metadata"`
}

type DeviceRecord struct {
	Profile     string         `json:"profile"`
	Kind        string         `json:"kind"`
	NodeID      string         `json:"node_id"`
	DeviceURA   string         `json:"device_ura"`
	State       string         `json:"state"`
	Online      bool           `json:"online"`
	IsSelf      bool           `json:"is_self"`
	Paired      bool           `json:"paired"`
	TenantID    string         `json:"tenant_id"`
	HubEndpoint string         `json:"hub_endpoint"`
	ProbeStatus string         `json:"probe_status"`
	ProbeError  any            `json:"probe_error"`
	LatencyMS   int            `json:"latency_ms"`
	Abilities   []string       `json:"abilities"`
	Metadata    map[string]any `json:"metadata"`
}

type AgentRecord struct {
	Name      string         `json:"name"`
	AgentURA  *string        `json:"agent_ura"`
	OwnerURA  *string        `json:"owner_ura"`
	DeviceURA *string        `json:"device_ura"`
	State     string         `json:"state"`
	Runtime   string         `json:"runtime"`
	Model     *string        `json:"model"`
	Label     *string        `json:"label"`
	Abilities []string       `json:"abilities"`
	Metadata  map[string]any `json:"metadata"`
}

type AbilityRecord struct {
	Profile           string         `json:"profile"`
	Kind              string         `json:"kind"`
	Name              string         `json:"name"`
	AbilityURA        string         `json:"ability_ura"`
	OwnerURA          string         `json:"owner_ura"`
	DescriptorRef     *string        `json:"descriptor_ref"`
	DescriptorVersion string         `json:"descriptor_version"`
	Visibility        string         `json:"visibility"`
	Class             string         `json:"class"`
	Description       string         `json:"description"`
	Source            string         `json:"source"`
	SchemaSummary     map[string]any `json:"schema_summary"`
	Hints             map[string]any `json:"hints"`
	Metadata          map[string]any `json:"metadata"`
}

type DevicePage struct {
	Profile    string         `json:"profile"`
	Kind       string         `json:"kind"`
	ItemKind   string         `json:"item_kind"`
	Items      []DeviceRecord `json:"items"`
	NextCursor *string        `json:"next_cursor"`
	Limit      int            `json:"limit"`
	Source     string         `json:"source"`
	Metadata   map[string]any `json:"metadata"`
}

type AgentPage struct {
	Profile    string         `json:"profile"`
	Kind       string         `json:"kind"`
	ItemKind   string         `json:"item_kind"`
	Items      []AgentRecord  `json:"items"`
	NextCursor *string        `json:"next_cursor"`
	Limit      int            `json:"limit"`
	Source     string         `json:"source"`
	Metadata   map[string]any `json:"metadata"`
}

type AbilityPage struct {
	Profile    string          `json:"profile"`
	Kind       string          `json:"kind"`
	ItemKind   string          `json:"item_kind"`
	Items      []AbilityRecord `json:"items"`
	NextCursor *string         `json:"next_cursor"`
	Limit      int             `json:"limit"`
	Source     string          `json:"source"`
	Metadata   map[string]any  `json:"metadata"`
}

func NewResolvedRefFromJSON(raw []byte) (ResolvedRef, error) {
	var ref ResolvedRef
	if err := json.Unmarshal(raw, &ref); err != nil {
		return ResolvedRef{}, invalidRuntimePayload(fmt.Sprintf("decode resolved ref JSON: %v", err), err)
	}
	if ref.Profile != directoryIdentityProfile || ref.Kind != "resolved_ref" || ref.AnswerKind == "" {
		return ResolvedRef{}, invalidRuntimePayload("invalid directory resolved_ref projection", nil)
	}
	if ref.Metadata == nil {
		ref.Metadata = map[string]any{}
	}
	return ref, nil
}

func NewDevicePageFromJSON(raw []byte) (DevicePage, error) {
	var page DevicePage
	if err := json.Unmarshal(raw, &page); err != nil {
		return DevicePage{}, invalidRuntimePayload(fmt.Sprintf("decode device page JSON: %v", err), err)
	}
	if err := validateDirectoryPage(page.Profile, page.Kind, page.ItemKind, page.Source, page.Limit, "device_page", "device"); err != nil {
		return DevicePage{}, err
	}
	return page, nil
}

func NewAgentPageFromJSON(raw []byte) (AgentPage, error) {
	var page AgentPage
	if err := json.Unmarshal(raw, &page); err != nil {
		return AgentPage{}, invalidRuntimePayload(fmt.Sprintf("decode agent page JSON: %v", err), err)
	}
	if err := validateDirectoryPage(page.Profile, page.Kind, page.ItemKind, page.Source, page.Limit, "agent_page", "agent"); err != nil {
		return AgentPage{}, err
	}
	return page, nil
}

func NewAbilityPageFromJSON(raw []byte) (AbilityPage, error) {
	var page AbilityPage
	if err := json.Unmarshal(raw, &page); err != nil {
		return AbilityPage{}, invalidRuntimePayload(fmt.Sprintf("decode ability page JSON: %v", err), err)
	}
	if err := validateDirectoryPage(page.Profile, page.Kind, page.ItemKind, page.Source, page.Limit, "ability_page", "ability"); err != nil {
		return AbilityPage{}, err
	}
	return page, nil
}

func marshalDirectoryPageQuery(query DirectoryQueryBase) ([]byte, error) {
	query = normalizeDirectoryPageQuery(query)
	if err := validateDirectoryQueryBase(query, true); err != nil {
		return nil, err
	}
	raw, err := json.Marshal(query)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode directory page query: %v", err), err)
	}
	return raw, nil
}

func normalizeDirectoryPageQuery(query DirectoryQueryBase) DirectoryQueryBase {
	if query.Limit == 0 {
		query.Limit = DefaultDirectoryPageSize
	}
	return query
}

func validateDirectoryQueryBase(query DirectoryQueryBase, requireLimit bool) error {
	if query.CallerURA == "" || query.CalleeURA == "" || query.SubjectURA == "" || query.DescriptorVersion == "" || query.NonceBase64 == "" {
		return invalidRuntimePayload("caller_ura, callee_ura, subject_ura, descriptor_version, and nonce_base64 are required", nil)
	}
	if query.CausalContext == nil {
		return invalidRuntimePayload("causal_context is required", nil)
	}
	if requireLimit {
		if query.Limit < 1 || query.Limit > MaxDirectoryPageSize {
			return invalidRuntimePayload("directory page limit exceeds bounds", nil)
		}
	}
	return nil
}

func validateDirectoryPage(profile string, kind string, itemKind string, source string, limit int, wantKind string, wantItem string) error {
	if profile != directoryIdentityProfile || kind != wantKind || itemKind != wantItem || source != directoryReadModelSource {
		return invalidRuntimePayload("invalid directory page projection", nil)
	}
	if limit < 1 || limit > MaxDirectoryPageSize {
		return invalidRuntimePayload("directory page limit exceeds bounds", nil)
	}
	return nil
}

func wrapDirectoryTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
